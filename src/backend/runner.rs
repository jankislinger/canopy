use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use std::{
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    time::Duration,
};

/// Owned by the UI; dropping it cancels even when the event loop returns an error.
#[derive(Default)]
pub struct CommandCancellation(Arc<AtomicBool>);

impl CommandCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

impl Drop for CommandCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Runs a command with cancellable output delivery and process cleanup.
pub fn execute(
    path: &Path,
    command: &[&str],
    cancelled: &AtomicBool,
    mut on_output: impl FnMut(String) -> bool,
) -> Result<(String, bool), Box<dyn std::error::Error + Send + Sync>> {
    if cancelled.load(Ordering::Relaxed) {
        return Ok((String::new(), false));
    }
    let pair = native_pty_system().openpty(PtySize {
        rows: 24,
        cols: 120,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut reader = pair.master.try_clone_reader()?;
    let mut builder = CommandBuilder::new(command[0]);
    builder.args(&command[1..]);
    builder.cwd(path);
    let mut child = pair.slave.spawn_command(builder)?;
    drop(pair.slave);

    // A bounded reader channel lets the worker check cancellation even when a
    // command produces no output. Dropping the receiver releases queued sends.
    let (output_tx, output_rx) = mpsc::sync_channel(8);
    let reader_task = std::thread::Builder::new().name("command-output".into()).spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            let result = reader.read(&mut buffer).map(|count| buffer[..count].to_vec());
            let finished = match &result {
                Ok(bytes) => bytes.is_empty(),
                Err(_) => true,
            };
            if output_tx.send(result).is_err() || finished {
                break;
            }
        }
    });
    if let Err(error) = reader_task {
        terminate(child.as_mut())?;
        return Err(error.into());
    }

    let mut output = String::new();
    let result = (|| {
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Ok(None);
            }
            match output_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(bytes)) if bytes.is_empty() => break,
                Ok(Ok(bytes)) => {
                    let chunk = String::from_utf8_lossy(&bytes).into_owned();
                    output.push_str(&chunk);
                    if !on_output(chunk) {
                        return Ok(None);
                    }
                }
                Ok(Err(error)) => return Err(error),
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        // A process can close output before exiting; waiting must remain cancellable.
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Ok(None);
            }
            if let Some(status) = child.try_wait()? {
                return Ok(Some(status.success()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })();
    // Normal completion has already reaped the child. On cancellation or read
    // failure, kill the command and its ordinary descendants before reaping it.
    if !matches!(result, Ok(Some(_))) {
        terminate(child.as_mut())?;
    }
    Ok((output, result?.unwrap_or(false)))
}

fn terminate(child: &mut dyn Child) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(pid) = child.process_id() {
        // portable-pty establishes a new session/process group for its child.
        // SIGKILL also handles commands that ignore hangup or termination.
        let result = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        child.wait()?;
        return Ok(());
    }
    if child.try_wait()?.is_none() {
        child.kill()?;
        child.wait()?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, time::Instant};

    #[test]
    fn streams_output_and_preserves_exit_status() {
        let cancelled = AtomicBool::new(false);
        let mut streamed = String::new();
        let (output, success) = execute(
            Path::new("/tmp"),
            &["sh", "-c", "printf hello; exit 7"],
            &cancelled,
            |chunk| {
                streamed.push_str(&chunk);
                true
            },
        )
        .unwrap();
        assert_eq!(output, "hello");
        assert_eq!(streamed, output);
        assert!(!success);
    }

    #[test]
    fn dropping_cancellation_stops_a_silent_command_promptly() {
        let cancellation = CommandCancellation::new();
        let flag = cancellation.flag();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            tx.send(execute(Path::new("/tmp"), &["sh", "-c", "sleep 3"], &flag, |_| true)).unwrap();
        });
        std::thread::sleep(Duration::from_millis(100));
        let start = Instant::now();
        drop(cancellation);
        let (_, success) = rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        assert!(!success);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn cancellation_kills_descendants_that_ignore_hangup() {
        let path = std::env::temp_dir().join(format!("canopy-cancel-test-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        let cancelled = AtomicBool::new(false);
        let start = Instant::now();
        let (_, success) = execute(
            &path,
            &["sh", "-c", "trap '' HUP TERM; (sleep 0.5; touch survived) & echo ready; wait"],
            &cancelled,
            |_| {
                cancelled.store(true, Ordering::Relaxed);
                true
            },
        )
        .unwrap();
        assert!(!success);
        assert!(start.elapsed() < Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(650));
        assert!(!path.join("survived").exists());
        fs::remove_dir_all(path).unwrap();
    }
}

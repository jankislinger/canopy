use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// A tmux session known to the local tmux server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub name: String,
    pub attached_clients: usize,
    pub working_directory: PathBuf,
    pub windows: Vec<Window>,
}

/// A window and its active pane in a project session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub active: bool,
    pub pane_count: usize,
    pub command: String,
    pub working_directory: PathBuf,
}

/// Errors returned by the tmux session backend.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("tmux is not available: {0}")]
    Unavailable(String),
    #[error("tmux command failed: {0}")]
    Command(String),
    #[error("cannot switch tmux sessions because the application is not running inside tmux")]
    OutsideTmux,
    #[error("invalid tmux session listing: {0}")]
    InvalidListing(String),
}

/// Provides operations against the local tmux server.
#[derive(Clone, Debug)]
pub struct SessionBackend {
    executable: PathBuf,
}

impl Default for SessionBackend {
    fn default() -> Self {
        Self::new("tmux")
    }
}

impl SessionBackend {
    /// Creates a backend using the supplied tmux executable.
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self { executable: executable.into() }
    }

    /// Lists sessions currently known to the tmux server.
    pub fn list(&self) -> Result<Vec<Session>, SessionError> {
        let output = self.command([
            "list-sessions",
            "-F",
            "#{session_name}\t#{session_attached}\t#{session_path}",
        ])?;
        let mut sessions = Session::parse_listing(&output)?;
        if !sessions.is_empty() {
            let windows = self.command([
                "list-windows",
                "-a",
                "-F",
                "#{session_name}\t#{window_index}\t#{window_id}\t#{window_active}\t#{window_panes}\t#{pane_current_command}\t#{window_name}\t#{pane_current_path}",
            ])?;
            for line in windows.lines().filter(|line| !line.trim().is_empty()) {
                let (name, window) = Window::parse_line(line)?;
                if let Some(session) = sessions.iter_mut().find(|session| session.name == name) {
                    session.windows.push(window);
                }
            }
            for session in &mut sessions {
                session.windows.sort_by_key(|window| window.index);
            }
        }
        Ok(sessions)
    }

    /// Creates a detached project session rooted at `project_path`.
    ///
    /// The first two tmux windows run the supplied editor and agent commands,
    /// followed by a shell window and a `git` window running `lazygit`.
    pub fn create(
        &self,
        project_path: &Path,
        editor_command: &str,
        agent_command: &str,
    ) -> Result<String, SessionError> {
        let path = project_path.canonicalize().map_err(|error| {
            SessionError::Command(format!("cannot resolve {}: {error}", project_path.display()))
        })?;
        let name = Session::name_for_path(&path);
        self.command_owned(vec![
            "new-session".into(),
            "-d".into(),
            "-s".into(),
            name.clone(),
            "-n".into(),
            "editor".into(),
            "-c".into(),
            path.display().to_string(),
            editor_command.into(),
        ])?;
        self.command_owned(vec![
            "new-window".into(),
            "-t".into(),
            name.clone(),
            "-n".into(),
            "agent".into(),
            "-c".into(),
            path.display().to_string(),
            agent_command.into(),
        ])?;
        self.command_owned(vec![
            "new-window".into(),
            "-t".into(),
            name.clone(),
            "-n".into(),
            "terminal".into(),
            "-c".into(),
            path.display().to_string(),
        ])?;
        self.command_owned(vec![
            "new-window".into(),
            "-t".into(),
            name.clone(),
            "-n".into(),
            "git".into(),
            "-c".into(),
            path.display().to_string(),
            "lazygit".into(),
        ])?;
        // `new-window` activates the window it creates; start from the first one.
        self.command_owned(vec!["select-window".into(), "-t".into(), format!("={name}:^")])?;
        Ok(name)
    }

    /// Switches to the exact session, preserving its active window and pane.
    pub fn switch_to(&self, session_name: &str) -> Result<(), SessionError> {
        if std::env::var_os("TMUX").is_none() {
            return Err(SessionError::OutsideTmux);
        }
        self.command_owned(vec!["switch-client".into(), "-t".into(), format!("={session_name}")])
            .map(|_| ())
    }

    /// Switches the current client to a specific window in an exact session.
    pub fn switch_to_window(
        &self,
        session_name: &str,
        window_id: &str,
    ) -> Result<(), SessionError> {
        if std::env::var_os("TMUX").is_none() {
            return Err(SessionError::OutsideTmux);
        }
        self.command_owned(vec![
            "switch-client".into(),
            "-t".into(),
            format!("={session_name}:{window_id}"),
        ])
        .map(|_| ())
    }

    /// Stops and removes a tmux session.
    pub fn stop(&self, session_name: &str) -> Result<(), SessionError> {
        self.command_owned(vec!["kill-session".into(), "-t".into(), session_name.into()])
            .map(|_| ())
    }

    fn command<'a, I>(&self, args: I) -> Result<String, SessionError>
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.command_owned(args.into_iter().map(str::to_owned).collect())
    }
    fn command_owned(&self, args: Vec<String>) -> Result<String, SessionError> {
        let output = Command::new(&self.executable)
            .args(args)
            .output()
            .map_err(|error| SessionError::Unavailable(error.to_string()))?;
        if !output.status.success() {
            return Err(SessionError::Command(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

impl Session {
    /// Derives a readable tmux session name from a project path.
    ///
    /// A repository root is named after its directory. A project below a Git
    /// repository is named `repository/project`.
    pub fn name_for_path(path: &Path) -> String {
        let project_name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
        let mut current = Some(path);
        while let Some(directory) = current {
            if directory.join(".git").is_dir() {
                let repository_name =
                    directory.file_name().unwrap_or(directory.as_os_str()).to_string_lossy();
                return if directory == path {
                    sanitize_name(&repository_name)
                } else {
                    format!("{}/{}", sanitize_name(&repository_name), sanitize_name(&project_name))
                };
            }
            current = directory.parent();
        }
        sanitize_name(&project_name)
    }

    fn parse_listing(listing: &str) -> Result<Vec<Self>, SessionError> {
        listing
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let mut fields = line.splitn(3, '\t');
                let name =
                    fields.next().ok_or_else(|| SessionError::InvalidListing(line.into()))?;
                let attached = fields
                    .next()
                    .and_then(|value| value.parse().ok())
                    .ok_or_else(|| SessionError::InvalidListing(line.into()))?;
                let path = fields
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| SessionError::InvalidListing(line.into()))?;
                Ok(Self {
                    name: name.into(),
                    attached_clients: attached,
                    working_directory: path.into(),
                    windows: Vec::new(),
                })
            })
            .collect()
    }
}

impl Window {
    fn parse_line(line: &str) -> Result<(&str, Self), SessionError> {
        let fields: Vec<_> = line.splitn(8, '\t').collect();
        let invalid = || SessionError::InvalidListing(line.into());
        if fields.len() != 8 || !fields[2].starts_with('@') {
            return Err(invalid());
        }
        Ok((
            fields[0],
            Self {
                index: fields[1].parse().map_err(|_| invalid())?,
                id: fields[2].into(),
                active: match fields[3] {
                    "0" => false,
                    "1" => true,
                    _ => return Err(invalid()),
                },
                pane_count: fields[4].parse().map_err(|_| invalid())?,
                command: fields[5].into(),
                name: fields[6].into(),
                working_directory: fields[7].into(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_names_use_repository_and_project_names() {
        let root = std::env::temp_dir().join("my-monorepo");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let project = root.join("my-utils");
        std::fs::create_dir_all(&project).unwrap();
        assert_eq!(Session::name_for_path(&root), "my-monorepo");
        assert_eq!(Session::name_for_path(&project), "my-monorepo/my-utils");
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn parses_session_listing() {
        let sessions =
            Session::parse_listing("editor\t1\t/home/jan/project\nidle\t0\t/home/jan/other\n")
                .unwrap();
        assert_eq!(
            sessions,
            vec![
                Session {
                    name: "editor".into(),
                    attached_clients: 1,
                    working_directory: "/home/jan/project".into(),
                    windows: Vec::new(),
                },
                Session {
                    name: "idle".into(),
                    attached_clients: 0,
                    working_directory: "/home/jan/other".into(),
                    windows: Vec::new(),
                }
            ]
        );
    }
    #[test]
    fn rejects_malformed_session_listing() {
        assert!(matches!(Session::parse_listing("broken"), Err(SessionError::InvalidListing(_))));
    }

    #[test]
    fn parses_window_index_and_active_pane_details() {
        let (session, window) =
            Window::parse_line("project\t12\t@42\t1\t3\tnvim\teditor\t/home/jan/my project")
                .unwrap();
        assert_eq!(session, "project");
        assert_eq!(window.index, 12);
        assert_eq!(window.id, "@42");
        assert!(window.active);
        assert_eq!(window.pane_count, 3);
        assert_eq!(window.command, "nvim");
        assert_eq!(window.name, "editor");
        assert_eq!(window.working_directory, PathBuf::from("/home/jan/my project"));
    }

    #[test]
    fn rejects_malformed_window_listing() {
        for line in [
            "broken",
            "project\tx\t@1\t1\t2\tsh\tterminal\t/tmp",
            "project\t0\t@1\t2\t2\tsh\tterminal\t/tmp",
        ] {
            assert!(matches!(Window::parse_line(line), Err(SessionError::InvalidListing(_))));
        }
    }

    #[cfg(unix)]
    #[test]
    fn switch_to_preserves_the_active_window() {
        use std::{
            fs,
            os::unix::fs::PermissionsExt,
            time::{SystemTime, UNIX_EPOCH},
        };
        // Run with a private environment rather than mutating TMUX while other
        // tests run concurrently. The fake executable rejects window targets.
        const EXECUTABLE_ENV: &str = "CANOPY_TEST_SESSION_SWITCH_EXECUTABLE";
        if let Some(executable) = std::env::var_os(EXECUTABLE_ENV) {
            SessionBackend::new(executable).switch_to("project").unwrap();
            return;
        }
        let temp = std::env::temp_dir().join(format!(
            "canopy-switch-test-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&temp).unwrap();
        let executable = temp.join("tmux");
        fs::write(&executable, "#!/bin/sh\n[ \"$#\" = 3 ] && [ \"$1\" = switch-client ] && [ \"$2\" = -t ] && [ \"$3\" = '=project' ]\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "backend::sessions::tests::switch_to_preserves_the_active_window"])
            .env("TMUX", "/tmp/canopy-test-socket,1,0")
            .env(EXECUTABLE_ENV, &executable)
            .output()
            .unwrap();
        fs::remove_dir_all(temp).unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    #[test]
    fn lists_and_sorts_windows_in_their_own_sessions() {
        use std::{
            fs,
            os::unix::fs::PermissionsExt,
            time::{SystemTime, UNIX_EPOCH},
        };
        let temp = std::env::temp_dir().join(format!(
            "canopy-windows-test-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&temp).unwrap();
        let executable = temp.join("tmux");
        fs::write(&executable, concat!(
            "#!/bin/sh\ncase \"$1\" in\n",
            "list-sessions) printf 'project\\t1\\t/tmp/project\\nother\\t0\\t/tmp/other\\n' ;;\n",
            "list-windows) [ \"$2\" = -a ] || exit 1\n",
            "printf 'project\\t3\\t@2\\t0\\t1\\tcodex\\tagent\\t/tmp/project\\n",
            "other\\t0\\t@3\\t1\\t2\\tbash\\tterminal\\t/tmp/other\\n",
            "project\\t1\\t@1\\t1\\t1\\tnvim\\teditor\\t/tmp/project\\n' ;;\n",
            "*) exit 1 ;;\nesac\n",
        )).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let sessions = SessionBackend::new(executable).list().unwrap();
        assert_eq!(
            sessions[0].windows.iter().map(|window| window.index).collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(sessions[0].windows[0].name, "editor");
        assert_eq!(sessions[1].windows[0].name, "terminal");
        assert_eq!(sessions[1].windows[0].index, 0);
        fs::remove_dir_all(temp).unwrap();
    }
}

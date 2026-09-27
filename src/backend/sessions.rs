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
        Self {
            executable: executable.into(),
        }
    }

    /// Lists sessions currently known to the tmux server.
    pub fn list(&self) -> Result<Vec<Session>, SessionError> {
        let output = self.command([
            "list-sessions",
            "-F",
            "#{session_name}\t#{session_attached}\t#{session_path}",
        ])?;
        parse_session_listing(&output)
    }

    /// Creates a detached project session rooted at `project_path`.
    ///
    /// The first tmux window runs `nvim`; the second is an empty shell window.
    pub fn create(&self, project_path: &Path) -> Result<String, SessionError> {
        let path = project_path.canonicalize().map_err(|error| {
            SessionError::Command(format!(
                "cannot resolve {}: {error}",
                project_path.display()
            ))
        })?;
        let name = session_name(&path);
        self.command_owned(vec![
            "new-session".into(),
            "-d".into(),
            "-s".into(),
            name.clone(),
            "-n".into(),
            "editor".into(),
            "-c".into(),
            path.display().to_string(),
            "nvim".into(),
        ])?;
        self.command_owned(vec![
            "new-window".into(),
            "-t".into(),
            name.clone(),
            "-n".into(),
            "codex".into(),
            "-c".into(),
            path.display().to_string(),
            "codex".into(),
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
        Ok(name)
    }

    /// Switches the current tmux client to `session_name`.
    pub fn switch_to(&self, session_name: &str) -> Result<(), SessionError> {
        if std::env::var_os("TMUX").is_none() {
            return Err(SessionError::OutsideTmux);
        }
        self.command_owned(vec![
            "switch-client".into(),
            "-t".into(),
            format!("{session_name}:editor"),
        ])
        .map(|_| ())
    }

    /// Stops and removes a tmux session.
    pub fn stop(&self, session_name: &str) -> Result<(), SessionError> {
        self.command_owned(vec![
            "kill-session".into(),
            "-t".into(),
            session_name.into(),
        ])
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

/// Derives a readable tmux session name from a project path.
///
/// A repository root is named after its directory. A project below a Git
/// repository is named `repository/project`.
pub fn session_name(path: &Path) -> String {
    let project_name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    let mut current = Some(path);
    while let Some(directory) = current {
        if directory.join(".git").is_dir() {
            let repository_name = directory
                .file_name()
                .unwrap_or(directory.as_os_str())
                .to_string_lossy();
            return if directory == path {
                sanitize_name(&repository_name)
            } else {
                format!(
                    "{}/{}",
                    sanitize_name(&repository_name),
                    sanitize_name(&project_name)
                )
            };
        }
        current = directory.parent();
    }
    sanitize_name(&project_name)
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

fn parse_session_listing(listing: &str) -> Result<Vec<Session>, SessionError> {
    listing
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.splitn(3, '\t');
            let name = fields
                .next()
                .ok_or_else(|| SessionError::InvalidListing(line.into()))?;
            let attached = fields
                .next()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| SessionError::InvalidListing(line.into()))?;
            let path = fields
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| SessionError::InvalidListing(line.into()))?;
            Ok(Session {
                name: name.into(),
                attached_clients: attached,
                working_directory: path.into(),
            })
        })
        .collect()
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
        assert_eq!(session_name(&root), "my-monorepo");
        assert_eq!(session_name(&project), "my-monorepo/my-utils");
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn parses_session_listing() {
        let sessions =
            parse_session_listing("editor\t1\t/home/jan/project\nidle\t0\t/home/jan/other\n")
                .unwrap();
        assert_eq!(
            sessions,
            vec![
                Session {
                    name: "editor".into(),
                    attached_clients: 1,
                    working_directory: "/home/jan/project".into()
                },
                Session {
                    name: "idle".into(),
                    attached_clients: 0,
                    working_directory: "/home/jan/other".into()
                }
            ]
        );
    }
    #[test]
    fn rejects_malformed_session_listing() {
        assert!(matches!(
            parse_session_listing("broken"),
            Err(SessionError::InvalidListing(_))
        ));
    }
}

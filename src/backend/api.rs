use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectKind {
    Git,
    Python,
    Rust,
}
impl ProjectKind {
    /// Returns the label shown in the project tree.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Git => "Git",
            Self::Python => "Python",
            Self::Rust => "Rust",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub path: PathBuf,
    pub kinds: Vec<ProjectKind>,
}
impl Project {
    /// Detects a project from its Git, Python, and Rust markers, if any are present.
    pub fn try_from_path(path: impl Into<PathBuf>) -> Option<Self> {
        let path = path.into();
        let mut kinds = Vec::new();
        if path.join(".git").is_dir() {
            kinds.push(ProjectKind::Git);
        }
        if path.join("pyproject.toml").is_file() {
            kinds.push(ProjectKind::Python);
        }
        if path.join("Cargo.toml").is_file() {
            kinds.push(ProjectKind::Rust);
        }
        (!kinds.is_empty()).then_some(Self { path, kinds })
    }
}
#[derive(Clone, Debug)]
pub struct Repository {
    pub path: PathBuf,
    pub projects: Vec<Project>,
}
pub const DEFAULT_SKIPPED_DIRS: [&str; 6] =
    ["target", "node_modules", ".venv", "__pycache__", ".cache", "build"];

/// Returns the current user's home directory from the `HOME` environment variable.
pub fn home_dir() -> color_eyre::Result<PathBuf> {
    env::var_os("HOME").map(PathBuf::from).ok_or_else(|| color_eyre::eyre::eyre!("HOME is not set"))
}

impl Repository {
    /// Discovers repositories while skipping directories whose names occur in `skipped_dirs`.
    pub fn discover(root: &Path, skipped_dirs: &[String]) -> color_eyre::Result<Vec<Self>> {
        let mut found = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(directory) = stack.pop() {
            let entries = match fs::read_dir(&directory) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let mut children = Vec::new();
            let mut is_repo = false;
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == ".git" && path.is_dir() {
                    is_repo = true;
                    continue;
                }
                if path.is_dir() && !name.starts_with('.') && !is_skipped(&name, skipped_dirs) {
                    children.push(path)
                }
            }
            if is_repo {
                found.push(Self::from_path(directory, skipped_dirs))
            }
            stack.extend(children)
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(found)
    }

    /// Creates the project group and project metadata for one Git repository.
    fn from_path(path: PathBuf, skipped_dirs: &[String]) -> Self {
        let root =
            Project::try_from_path(path.clone()).expect("repository root must have a Git marker");
        let mut projects = vec![root];
        let mut stack = vec![path.clone()];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = fs::read_dir(&directory) else { continue };
            for entry in entries.flatten() {
                let child = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if !child.is_dir() || name.starts_with('.') || is_skipped(&name, skipped_dirs) {
                    continue;
                }

                // Nested Git repositories are discovered independently by the outer scan.
                if child.join(".git").exists() {
                    continue;
                }

                if let Some(project) = Project::try_from_path(child.clone()) {
                    projects.push(project)
                }
                stack.push(child);
            }
        }
        projects.sort_by(|a, b| a.path.cmp(&b.path));
        Self { path, projects }
    }
}

fn is_skipped(name: &str, skipped_dirs: &[String]) -> bool {
    skipped_dirs.iter().any(|skipped| skipped == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = env::temp_dir().join(format!(
                "canopy-test-{}",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn detects_repository_and_direct_projects() {
        let t = Temp::new();
        let r = t.0.join("repo");
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::write(r.join("Cargo.toml"), "").unwrap();
        let p = r.join("python");
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("pyproject.toml"), "").unwrap();
        let skipped_dirs =
            DEFAULT_SKIPPED_DIRS.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>();
        let repos = Repository::discover(&t.0, &skipped_dirs).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].projects.len(), 2);
        assert_eq!(repos[0].projects[0].kinds, vec![ProjectKind::Git, ProjectKind::Rust]);
        assert_eq!(repos[0].projects[1].kinds, vec![ProjectKind::Python]);

        let filtered = Repository::discover(&t.0, &["python".into()]).unwrap();
        assert_eq!(filtered[0].projects.len(), 1);
    }
}

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
#[derive(Clone, Debug)]
pub struct Project {
    pub path: PathBuf,
    pub kinds: Vec<ProjectKind>,
}
#[derive(Clone, Debug)]
pub struct Repository {
    pub path: PathBuf,
    pub projects: Vec<Project>,
}
const SKIPPED_DIRS: [&str; 6] =
    ["target", "node_modules", ".venv", "__pycache__", ".cache", "build"];

/// Returns the current user's home directory from the `HOME` environment variable.
pub fn home_dir() -> color_eyre::Result<PathBuf> {
    env::var_os("HOME").map(PathBuf::from).ok_or_else(|| color_eyre::eyre::eyre!("HOME is not set"))
}

/// Discovers Git repositories and their direct child Python/Rust projects below `root`.
///
/// Hidden directories and common generated or dependency directories are skipped.
pub fn discover_repositories(root: &Path) -> color_eyre::Result<Vec<Repository>> {
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
            if path.is_dir() && !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_str()) {
                children.push(path)
            }
        }
        if is_repo {
            found.push(make_repository(directory))
        }
        stack.extend(children)
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// Creates the project group and project metadata for one Git repository.
fn make_repository(path: PathBuf) -> Repository {
    let mut root_kinds = vec![ProjectKind::Git];
    if path.join("pyproject.toml").is_file() {
        root_kinds.push(ProjectKind::Python)
    }
    if path.join("Cargo.toml").is_file() {
        root_kinds.push(ProjectKind::Rust)
    }
    let mut projects = vec![Project { path: path.clone(), kinds: root_kinds }];
    if let Ok(entries) = fs::read_dir(&path) {
        for entry in entries.flatten() {
            let child = entry.path();
            if !child.is_dir() {
                continue;
            }
            let mut kinds = Vec::new();
            if child.join("pyproject.toml").is_file() {
                kinds.push(ProjectKind::Python)
            }
            if child.join("Cargo.toml").is_file() {
                kinds.push(ProjectKind::Rust)
            }
            if !kinds.is_empty() {
                projects.push(Project { path: child, kinds })
            }
        }
    }
    projects.sort_by(|a, b| a.path.cmp(&b.path));
    Repository { path, projects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = env::temp_dir().join(format!(
                "learning-tui-test-{}",
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
        let repos = discover_repositories(&t.0).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].projects.len(), 2);
        assert_eq!(repos[0].projects[0].kinds, vec![ProjectKind::Git, ProjectKind::Rust]);
        assert_eq!(repos[0].projects[1].kinds, vec![ProjectKind::Python]);
    }
}

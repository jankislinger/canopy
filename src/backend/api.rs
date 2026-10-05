use super::commands::CommandKind;
use regex::Regex;
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

static JUST_RECIPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\w+):([^=]|$)").expect("valid Just recipe pattern"));

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
    kinds: Vec<ProjectKind>,
    just_recipes: Option<Vec<String>>,
}
impl Project {
    /// Creates a project from its path, kinds, and cached Just recipes.
    pub fn new(path: PathBuf, kinds: Vec<ProjectKind>, just_recipes: Option<Vec<String>>) -> Self {
        Self { path, kinds, just_recipes }
    }

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
        if kinds.is_empty() {
            return None;
        }
        let just_recipes = fs::read_to_string(path.join("justfile")).ok().map(|contents| {
            contents
                .lines()
                .filter_map(|line| {
                    JUST_RECIPE.captures(line).map(|captures| captures[1].to_owned())
                })
                .collect()
        });
        Some(Self::new(path, kinds, just_recipes))
    }

    /// Returns the detected project kinds.
    pub fn kinds(&self) -> &[ProjectKind] {
        &self.kinds
    }

    /// Selects Just recipes or language defaults for this project.
    pub fn commands(&self, command: CommandKind) -> Vec<Vec<&'static str>> {
        let has_recipe = self
            .just_recipes
            .as_deref()
            .is_some_and(|recipes| recipes.iter().any(|recipe| recipe == command.recipe()));
        if has_recipe {
            return vec![vec!["just", command.recipe()]];
        }

        let (python, rust) = match command {
            CommandKind::Test => (vec![vec!["uv", "run", "pytest"]], vec![vec!["cargo", "test"]]),
            CommandKind::Lint => (
                vec![
                    vec!["uv", "run", "ruff", "format", "--check"],
                    vec!["uv", "run", "ruff", "check"],
                ],
                vec![vec!["cargo", "fmt", "--check"], vec!["cargo", "clippy"]],
            ),
        };

        let mut commands = Vec::new();
        if self.kinds.contains(&ProjectKind::Python) {
            commands.extend(python);
        }
        if self.kinds.contains(&ProjectKind::Rust) {
            commands.extend(rust);
        }
        commands
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
    fn caches_only_strict_no_parameter_recipe_headers() {
        let t = Temp::new();
        fs::write(t.0.join("Cargo.toml"), "").unwrap();
        fs::write(
            t.0.join("justfile"),
            concat!(
                "test:\n",
                "lint: test\n",
                "_helper:\r\n",
                "# commented:\n",
                "    body:\n",
                "assignment:=\"value\"\n",
                "required arg:\n",
                "optional arg='value':\n",
                "variadic *args:\n",
                "test-unit:\n",
                "@quiet:\n",
                "spaced :\n",
            ),
        )
        .unwrap();
        let project = Project::try_from_path(t.0.clone()).unwrap();
        assert_eq!(
            project.just_recipes,
            Some(vec!["test".into(), "lint".into(), "_helper".into()])
        );
    }

    #[test]
    fn distinguishes_missing_unreadable_and_empty_justfiles() {
        let t = Temp::new();
        fs::write(t.0.join("Cargo.toml"), "").unwrap();
        assert_eq!(Project::try_from_path(t.0.clone()).unwrap().just_recipes, None);

        // A directory at the file path reliably causes a read failure, even when run as root.
        fs::create_dir(t.0.join("justfile")).unwrap();
        assert_eq!(Project::try_from_path(t.0.clone()).unwrap().just_recipes, None);
        fs::remove_dir(t.0.join("justfile")).unwrap();

        for contents in ["", "# test:\nonly_with_args arg:\n"] {
            fs::write(t.0.join("justfile"), contents).unwrap();
            assert_eq!(Project::try_from_path(t.0.clone()).unwrap().just_recipes, Some(vec![]));
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

#[cfg(test)]
mod command_tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TempProject(PathBuf);

    impl TempProject {
        fn project(&self, kinds: &[ProjectKind]) -> Project {
            fs::create_dir_all(self.0.join(".git")).unwrap();
            let mut project = Project::try_from_path(self.0.clone()).unwrap();
            project.kinds = kinds.to_vec();
            project
        }

        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "canopy-commands-test-{}",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn command_selection_uses_cached_recipes_until_project_is_recreated() {
        let directory = TempProject::new();
        let justfile = directory.0.join("justfile");
        fs::write(&justfile, "test:\n    true\n").unwrap();
        let project = directory.project(&[ProjectKind::Rust]);

        fs::write(&justfile, "lint:\n    true\n").unwrap();
        assert_eq!(project.commands(CommandKind::Test), vec![vec!["just", "test"]]);
        assert_eq!(
            project.commands(CommandKind::Lint),
            vec![vec!["cargo", "fmt", "--check"], vec!["cargo", "clippy"]]
        );

        let refreshed = directory.project(&[ProjectKind::Rust]);
        assert_eq!(refreshed.commands(CommandKind::Test), vec![vec!["cargo", "test"]]);
        assert_eq!(refreshed.commands(CommandKind::Lint), vec![vec!["just", "lint"]]);

        fs::remove_file(&justfile).unwrap();
        assert_eq!(project.commands(CommandKind::Test), vec![vec!["just", "test"]]);
        assert_eq!(refreshed.commands(CommandKind::Lint), vec![vec!["just", "lint"]]);
        assert_eq!(directory.project(&[ProjectKind::Rust]).just_recipes, None);
    }

    #[test]
    fn language_commands_keep_the_existing_defaults() {
        let kinds = [ProjectKind::Git, ProjectKind::Python, ProjectKind::Rust];
        let project = Project::new(PathBuf::from("/unused"), kinds.to_vec(), None);
        assert_eq!(
            project.commands(CommandKind::Test),
            vec![vec!["uv", "run", "pytest"], vec!["cargo", "test"]]
        );
        assert_eq!(
            project.commands(CommandKind::Lint),
            vec![
                vec!["uv", "run", "ruff", "format", "--check"],
                vec!["uv", "run", "ruff", "check"],
                vec!["cargo", "fmt", "--check"],
                vec!["cargo", "clippy"],
            ]
        );
    }

    #[test]
    fn justfile_commands_override_language_defaults_and_work_for_git_projects() {
        let project = TempProject::new();
        fs::write(project.0.join("justfile"), "test:\n    true\nlint:\n    true\n").unwrap();
        for kinds in
            [vec![ProjectKind::Git], vec![ProjectKind::Git, ProjectKind::Python, ProjectKind::Rust]]
        {
            assert_eq!(
                project.project(&kinds).commands(CommandKind::Test),
                vec![vec!["just", "test"]]
            );
            assert_eq!(
                project.project(&kinds).commands(CommandKind::Lint),
                vec![vec!["just", "lint"]]
            );
        }
    }

    #[test]
    fn missing_just_recipe_falls_back_to_language_commands() {
        let project = TempProject::new();
        let kinds = [ProjectKind::Rust];
        fs::write(project.0.join("justfile"), "test:\n    true\n# lint:\nlint-extra:\n    true\n")
            .unwrap();
        assert_eq!(project.project(&kinds).commands(CommandKind::Test), vec![vec!["just", "test"]]);
        assert_eq!(
            project.project(&kinds).commands(CommandKind::Lint),
            vec![vec!["cargo", "fmt", "--check"], vec!["cargo", "clippy"],]
        );

        fs::write(project.0.join("justfile"), "lint:\n    true\ntest-extra:\n    true\n").unwrap();
        assert_eq!(
            project.project(&kinds).commands(CommandKind::Test),
            vec![vec!["cargo", "test"]]
        );
        assert_eq!(project.project(&kinds).commands(CommandKind::Lint), vec![vec!["just", "lint"]]);
    }
}

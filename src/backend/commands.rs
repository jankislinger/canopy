use super::{Project, ProjectKind};

#[derive(Clone, Copy)]
pub enum CommandKind {
    Test,
    Lint,
}

impl CommandKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Test => "Test",
            Self::Lint => "Lint",
        }
    }

    fn recipe(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Lint => "lint",
        }
    }
}

impl Project {
    /// Selects Just recipes or language defaults for this project.
    pub fn commands(&self, command: CommandKind) -> Vec<Vec<&'static str>> {
        let declaration = format!("{}:", command.recipe());
        let has_recipe = std::fs::read_to_string(self.path.join("justfile"))
            .is_ok_and(|contents| contents.lines().any(|line| line.starts_with(&declaration)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TempProject(PathBuf);

    impl TempProject {
        fn project(&self, kinds: &[ProjectKind]) -> Project {
            Project { path: self.0.clone(), kinds: kinds.to_vec() }
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
    fn language_commands_keep_the_existing_defaults() {
        let project = TempProject::new();
        let kinds = [ProjectKind::Git, ProjectKind::Python, ProjectKind::Rust];
        assert_eq!(
            project.project(&kinds).commands(CommandKind::Test),
            vec![vec!["uv", "run", "pytest"], vec!["cargo", "test"]]
        );
        assert_eq!(
            project.project(&kinds).commands(CommandKind::Lint),
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

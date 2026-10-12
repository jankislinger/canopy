//! Recipe listings for the selected project's own justfile.
use std::{ffi::OsStr, path::Path, process::Command};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub name: String,
    pub description: String,
    pub display: String,
}

impl Recipe {
    pub fn parse_listing(listing: &str) -> Vec<Self> {
        listing
            .lines()
            .filter_map(|line| {
                let display = line.trim();
                let name = display.split_whitespace().next()?;
                if name.ends_with(':')
                    || !name.chars().all(|character| {
                        character.is_alphanumeric() || matches!(character, '_' | '-' | ':')
                    })
                {
                    return None;
                }
                let description = display
                    .split_once(" #")
                    .map(|(_, description)| description.trim())
                    .unwrap_or("");
                Some(Self {
                    name: name.into(),
                    description: description.into(),
                    display: display.into(),
                })
            })
            .collect()
    }

    pub fn matches(&self, query: &str) -> bool {
        matches_text(&format!("{} {}", self.name, self.description), query)
    }

    pub fn is_exact_match(&self, query: &str) -> bool {
        self.name.to_lowercase() == query.trim().to_lowercase()
    }

    pub fn is_name_prefix(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        !query.is_empty() && self.name.to_lowercase().starts_with(&query)
    }
}

fn matches_text(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query.split_whitespace().all(|word| {
        let mut characters = text.chars();
        word.to_lowercase().chars().all(|wanted| characters.any(|character| character == wanted))
    })
}

pub fn list_recipes(path: &Path) -> Result<Option<String>, String> {
    list_recipes_with(path, OsStr::new("just"))
}

pub fn justfile(path: &Path) -> Option<std::path::PathBuf> {
    ["justfile", "Justfile", ".justfile"]
        .into_iter()
        .map(|name| path.join(name))
        .find(|file| file.is_file())
}

/// Parses recipe arguments without evaluating shell expansions or operators.
pub fn recipe_command(justfile: &Path, input: &str) -> Result<Vec<String>, String> {
    let args = shell_words::split(input)
        .map_err(|_| "Close the quote or finish the escape before running".to_owned())?;
    if args.is_empty() {
        return Err("Enter a recipe name".into());
    }
    let mut command =
        vec!["just".into(), "--justfile".into(), justfile.to_string_lossy().into_owned()];
    command.extend(args);
    Ok(command)
}

fn list_recipes_with(path: &Path, executable: &OsStr) -> Result<Option<String>, String> {
    let Some(justfile) = justfile(path) else { return Ok(None) };
    let output = Command::new(executable)
        .current_dir(path)
        .args([
            "--list",
            "--color",
            "never",
            "--list-heading",
            "",
            "--list-prefix",
            "  ",
            "--justfile",
        ])
        .arg(justfile)
        .output()
        .map_err(|error| format!("could not run just: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).trim_end().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "canopy-just-list-{}",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_local_justfile_does_not_use_parent_or_invoke_just() {
        let temp = Temp::new();
        fs::write(temp.0.join("justfile"), "parent:\n    true\n").unwrap();
        let child = temp.0.join("child");
        fs::create_dir(&child).unwrap();
        assert_eq!(list_recipes_with(&child, OsStr::new("/nonexistent-canopy-just")), Ok(None));
    }

    #[cfg(unix)]
    #[test]
    fn listing_preserves_parameters_descriptions_and_aliases_without_running_recipes() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        fs::write(temp.0.join("justfile"), "deploy target:\n    touch ran-recipe\n").unwrap();
        let executable = temp.0.join("fake-just");
        fs::write(&executable, concat!(
            "#!/bin/sh\n",
            "[ \"$#\" = 9 ] && [ \"$1\" = --list ] && [ \"$3\" = never ] && [ \"$8\" = --justfile ] && [ \"$9\" = \"$PWD/justfile\" ] || exit 1\n",
            "printf '  deploy target  # Deploy to target\\n  release        # alias for deploy\\n'\n",
        )).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let listing = list_recipes_with(&temp.0, executable.as_os_str()).unwrap().unwrap();
        assert_eq!(
            listing,
            "  deploy target  # Deploy to target\n  release        # alias for deploy"
        );
        assert!(!temp.0.join("ran-recipe").exists());
    }

    #[test]
    fn reports_unavailable_just_when_a_justfile_exists() {
        let temp = Temp::new();
        fs::write(temp.0.join("Justfile"), "").unwrap();
        assert!(
            list_recipes_with(&temp.0, OsStr::new("/nonexistent-canopy-just"))
                .unwrap_err()
                .contains("could not run just")
        );
    }

    #[test]
    fn recipe_arguments_support_quotes_and_remain_literal() {
        let command = recipe_command(
            Path::new("/tmp/my project/justfile"),
            "deploy 'hello world' \"$(touch owned)\" 'é' a\\ b",
        )
        .unwrap();
        assert_eq!(
            command,
            [
                "just",
                "--justfile",
                "/tmp/my project/justfile",
                "deploy",
                "hello world",
                "$(touch owned)",
                "é",
                "a b"
            ]
        );
        assert!(recipe_command(Path::new("justfile"), "   ").is_err());
        assert!(recipe_command(Path::new("justfile"), "deploy 'unfinished").is_err());
    }

    #[test]
    fn fuzzy_search_matches_recipe_names_and_descriptions() {
        let recipes = Recipe::parse_listing(
            "[Deployment]\n  deploy target # Publish a release\n  test # Run checks\n  release # alias for deploy\n  # additional comment\n",
        );
        assert_eq!(recipes.len(), 3);
        assert!(recipes[0].matches("DPLY"));
        assert!(recipes[0].matches("pbl rls"));
        assert!(recipes[0].matches("dpl rls"));
        assert!(recipes[0].matches("rls pbl")); // Each word matches independently.
        assert!(!recipes[0].matches("checks"));
        assert!(recipes[1].matches("rn chks"));
        assert_eq!(recipes[2].name, "release");
    }
}

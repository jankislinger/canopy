use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
};

use crate::backend::api::DEFAULT_SKIPPED_DIRS;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    Hidden,
    Collapsed,
    Expanded,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DirectoryOverride {
    pub display: Option<DisplayMode>,
    pub editor: Option<String>,
    pub agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub skipped_dirs: Vec<String>,
    pub editor: String,
    pub agent: String,
    pub directory_overrides: HashMap<PathBuf, DirectoryOverride>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsFile {
    skipped_dirs: Option<Vec<String>>,
    extra_skipped_dirs: Option<Vec<String>>,
    editor: Option<String>,
    agent: Option<String>,
    #[serde(default)]
    directory_overrides: HashMap<String, DirectoryOverride>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            skipped_dirs: DEFAULT_SKIPPED_DIRS.iter().map(|name| (*name).to_owned()).collect(),
            editor: "nvim".into(),
            agent: "codex".into(),
            directory_overrides: HashMap::new(),
        }
    }
}

impl Settings {
    /// Loads user settings, returning defaults and a displayable error on invalid input.
    pub fn load(home: &Path) -> (Self, Option<String>) {
        let path = home.join(".config/canopy/settings.json");
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default(), None);
            }
            Err(error) => {
                return (Self::default(), Some(format!("{}: {error}", path.display())));
            }
        };

        match Self::parse(&contents, home) {
            Ok(settings) => (settings, None),
            Err(error) => (Self::default(), Some(error)),
        }
    }

    fn parse(contents: &str, home: &Path) -> Result<Self, String> {
        let file: SettingsFile =
            serde_json::from_str(contents).map_err(|error| error.to_string())?;
        if file.skipped_dirs.is_some() && file.extra_skipped_dirs.is_some() {
            return Err(
                "settings cannot contain both `skipped_dirs` and `extra_skipped_dirs`".into()
            );
        }

        let skipped_dirs = if let Some(names) = file.skipped_dirs {
            names
        } else {
            let mut names: Vec<String> =
                DEFAULT_SKIPPED_DIRS.iter().map(|name| (*name).to_owned()).collect();
            names.extend(file.extra_skipped_dirs.unwrap_or_default());
            names
        };

        let mut directory_overrides = HashMap::new();
        for (key, value) in file.directory_overrides {
            let path = resolve_override_path(&key, home)?;
            if directory_overrides.insert(path.clone(), value).is_some() {
                return Err(format!("multiple directory overrides resolve to {}", path.display()));
            }
        }

        Ok(Self {
            skipped_dirs,
            editor: file.editor.unwrap_or_else(|| "nvim".into()),
            agent: file.agent.unwrap_or_else(|| "codex".into()),
            directory_overrides,
        })
    }

    pub fn override_for(&self, path: &Path) -> Option<&DirectoryOverride> {
        self.directory_overrides.get(path)
    }

    pub fn editor_for(&self, path: &Path) -> &str {
        self.override_for(path)
            .and_then(|settings| settings.editor.as_deref())
            .unwrap_or(&self.editor)
    }

    pub fn agent_for(&self, path: &Path) -> &str {
        self.override_for(path)
            .and_then(|settings| settings.agent.as_deref())
            .unwrap_or(&self.agent)
    }

    pub fn display_for(&self, path: &Path) -> DisplayMode {
        self.override_for(path)
            .and_then(|settings| settings.display)
            .unwrap_or(DisplayMode::Expanded)
    }
}

fn resolve_override_path(key: &str, home: &Path) -> Result<PathBuf, String> {
    let path = if key == "~" {
        home.to_path_buf()
    } else if let Some(suffix) = key.strip_prefix("~/") {
        home.join(suffix)
    } else {
        let path = PathBuf::from(key);
        if !path.is_absolute() {
            return Err(format!(
                "directory override path must be absolute or start with `~/`: {key}"
            ));
        }
        path
    };
    Ok(normalize_path(path))
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempHome(PathBuf);

    impl TempHome {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "canopy-config-test-{}",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn parse(contents: &str, home: &Path) -> Result<Settings, String> {
        Settings::parse(contents, home)
    }

    #[test]
    fn missing_file_uses_defaults() {
        let home = TempHome::new();
        let (settings, error) = Settings::load(&home.0);
        assert_eq!(settings, Settings::default());
        assert!(error.is_none());
    }

    #[test]
    fn malformed_json_returns_raw_parser_error() {
        let error = parse("{ invalid", Path::new("/home/test")).unwrap_err();
        assert!(error.contains("key must be a string"));
    }

    #[test]
    fn wrong_types_and_unknown_fields_are_rejected() {
        assert!(parse(r#"{"editor": 4}"#, Path::new("/home/test")).is_err());
        assert!(parse(r#"{"unknown": true}"#, Path::new("/home/test")).is_err());
    }

    #[test]
    fn skipped_directory_settings_are_exclusive_and_compose_with_defaults() {
        assert!(
            parse(
                r#"{"skipped_dirs": [], "extra_skipped_dirs": ["vendor"]}"#,
                Path::new("/home/test")
            )
            .is_err()
        );
        let settings =
            parse(r#"{"extra_skipped_dirs": ["vendor"]}"#, Path::new("/home/test")).unwrap();
        assert!(settings.skipped_dirs.contains(&"vendor".into()));
        assert!(settings.skipped_dirs.contains(&"target".into()));
        let settings = parse(r#"{"skipped_dirs": ["custom"]}"#, Path::new("/home/test")).unwrap();
        assert_eq!(settings.skipped_dirs, ["custom"]);
    }

    #[test]
    fn resolves_override_paths_and_commands() {
        let settings = parse(
            r#"{
                "editor": "nvim --clean",
                "agent": "codex --full-auto",
                "directory_overrides": {
                    "~/my_dir": {
                        "display": "hidden",
                        "editor": "nvim --listen socket"
                    }
                }
            }"#,
            Path::new("/home/test"),
        )
        .unwrap();
        let project = Path::new("/home/test/my_dir");
        assert_eq!(settings.editor_for(project), "nvim --listen socket");
        assert_eq!(settings.agent_for(project), "codex --full-auto");
        assert_eq!(settings.display_for(project), DisplayMode::Hidden);
    }

    #[test]
    fn rejects_relative_override_paths_and_invalid_display_modes() {
        assert!(
            parse(
                r#"{"directory_overrides": {"my_dir": {"display": "hidden"}}}"#,
                Path::new("/home/test")
            )
            .is_err()
        );
        assert!(
            parse(
                r#"{"directory_overrides": {"~/my_dir": {"display": "visible"}}}"#,
                Path::new("/home/test")
            )
            .is_err()
        );
    }
}

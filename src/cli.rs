//! Flags that print and exit without starting the terminal UI, plus run options.

use std::path::{Path, PathBuf};

/// What `main` should do after reading the command line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Run(Invocation),
    Help,
    Version,
    Error(String),
}

/// Options for a normal launch. `root` limits the project listing; `None` means `$HOME`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Invocation {
    pub root: Option<String>,
    pub config_file: Option<String>,
}

pub(crate) const HELP: &str = "\
canopy — terminal project manager

Usage: canopy [OPTIONS] [PATH]

Arguments:
  [PATH]                 List projects under this directory (default: $HOME)

Options:
  -h, --help             Show this help and exit
  -V, --version          Print the build commit and exit
  --config-file <FILE>   Settings file (default: ~/.config/canopy/settings.json)
";

/// `Build version: <short sha>`, matching the startup label (`ef12345` or `ef12345-dirty`).
pub(crate) fn version_line(sha: &str, dirty: bool) -> String {
    let id = match (sha.get(..7), dirty) {
        (Some(short), true) => format!("{short}-dirty"),
        (Some(short), false) => short.to_owned(),
        (None, _) => "unknown".into(),
    };
    format!("Build version: {id}")
}

/// Expands a leading `~` against `home`. Other paths are left for the caller to resolve.
pub(crate) fn expand_home(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" {
        home.to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(raw)
    }
}

/// Parses flags and an optional directory. Help and version must be the only argument.
pub(crate) fn parse<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut root = None;
    let mut config_file = None;
    let mut help = false;
    let mut version = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        match arg {
            "-h" | "--help" => help = true,
            "-V" | "--version" => version = true,
            "--config-file" => {
                let Some(value) = args.next() else {
                    return Command::Error("missing argument for --config-file".into());
                };
                let value = value.as_ref();
                if value.starts_with('-') || value.is_empty() {
                    return Command::Error("missing argument for --config-file".into());
                }
                if config_file.replace(value.to_owned()).is_some() {
                    return Command::Error("option '--config-file' given more than once".into());
                }
            }
            value if value.starts_with("--config-file=") => {
                let value = &value["--config-file=".len()..];
                if value.is_empty() {
                    return Command::Error("missing argument for --config-file".into());
                }
                if config_file.replace(value.to_owned()).is_some() {
                    return Command::Error("option '--config-file' given more than once".into());
                }
            }
            value if value.starts_with('-') => {
                return Command::Error(format!("unrecognized option: {value}"));
            }
            value => {
                if root.replace(value.to_owned()).is_some() {
                    return Command::Error(format!("unexpected argument: {value}"));
                }
            }
        }
    }

    let exclusive = help || version;
    let has_run_args = root.is_some() || config_file.is_some();
    if help && version || exclusive && has_run_args {
        return Command::Error("unexpected argument".into());
    }
    if help {
        return Command::Help;
    }
    if version {
        return Command::Version;
    }
    Command::Run(Invocation { root, config_file })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(root: Option<&str>, config_file: Option<&str>) -> Command {
        Command::Run(Invocation {
            root: root.map(str::to_owned),
            config_file: config_file.map(str::to_owned),
        })
    }

    #[test]
    fn no_args_runs_the_tui() {
        assert_eq!(parse(std::iter::empty::<&str>()), run(None, None));
    }

    #[test]
    fn help_and_version_flags() {
        assert_eq!(parse(["-h"]), Command::Help);
        assert_eq!(parse(["--help"]), Command::Help);
        assert_eq!(parse(["-V"]), Command::Version);
        assert_eq!(parse(["--version"]), Command::Version);
    }

    #[test]
    fn path_and_config_file() {
        assert_eq!(parse(["."]), run(Some("."), None));
        assert_eq!(parse(["~/python-projects"]), run(Some("~/python-projects"), None));
        assert_eq!(
            parse(["--config-file", "settings.json", "."]),
            run(Some("."), Some("settings.json"))
        );
        assert_eq!(
            parse(["~/python-projects", "--config-file=~/other.json"]),
            run(Some("~/python-projects"), Some("~/other.json"))
        );
    }

    #[test]
    fn extra_or_unknown_args_are_errors() {
        assert!(matches!(parse(["--version", "--help"]), Command::Error(_)));
        assert!(matches!(parse(["-hv"]), Command::Error(_)));
        assert!(matches!(parse([".", "other"]), Command::Error(_)));
        assert!(matches!(parse(["--config-file"]), Command::Error(_)));
        assert!(matches!(parse(["--config-file", "--help"]), Command::Error(_)));
        assert!(matches!(parse(["--help", "."]), Command::Error(_)));
    }

    #[test]
    fn version_line_uses_the_short_commit_or_unknown() {
        assert_eq!(version_line("abcdef1234567890", false), "Build version: abcdef1");
        assert_eq!(version_line("abcdef1234567890", true), "Build version: abcdef1-dirty");
        assert_eq!(version_line("", false), "Build version: unknown");
    }

    #[test]
    fn expand_home_only_rewrites_a_leading_tilde() {
        let home = Path::new("/home/test");
        assert_eq!(expand_home("~", home), PathBuf::from("/home/test"));
        assert_eq!(
            expand_home("~/python-projects", home),
            PathBuf::from("/home/test/python-projects")
        );
        assert_eq!(expand_home(".", home), PathBuf::from("."));
        assert_eq!(expand_home("/tmp/not~home", home), PathBuf::from("/tmp/not~home"));
    }
}

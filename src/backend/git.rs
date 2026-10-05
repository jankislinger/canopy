use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Git information displayed for a selected project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitStatus {
    pub repository: PathBuf,
    pub branch: String,
    pub working_tree: WorkingTree,
    pub commits: Vec<GitCommit>,
}

/// Whether Git reports changes in the selected project, elsewhere, or nowhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkingTree {
    Clean,
    ChangesOutsideProject,
    ChangesInProject,
}

/// A compact entry from the repository history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitCommit {
    pub sha: String,
    pub author: String,
    pub authored_by_user: bool,
    pub message: String,
    pub age: String,
}

impl GitStatus {
    /// Loads branch and recent commit information for a project or its Git parent.
    pub fn load(path: &Path) -> Result<Self, String> {
        let repository = path
            .ancestors()
            .find(|candidate| candidate.join(".git").exists())
            .ok_or_else(|| "Not in a Git repository".to_string())?
            .to_path_buf();
        let branch = git(&repository, ["branch", "--show-current"])
            .map(|branch| if branch.is_empty() { "HEAD".into() } else { branch })?;
        let user_email = git(&repository, ["config", "--get", "user.email"]).unwrap_or_default();
        let user_name = git(&repository, ["config", "--get", "user.name"]).unwrap_or_default();
        let log = git(&repository, ["log", "-6", "--format=%H%x1f%an%x1f%ae%x1f%s%x1f%ar%x1e"])?;
        let commits = GitCommit::parse_log(&log, &user_email, &user_name);
        let all_changes = git(&repository, ["status", "--porcelain=v1", "--untracked-files=all"])?;
        let project_changes =
            git(path, ["status", "--porcelain=v1", "--untracked-files=all", "--", "."])?;
        let working_tree = if all_changes.is_empty() {
            WorkingTree::Clean
        } else if project_changes.is_empty() {
            WorkingTree::ChangesOutsideProject
        } else {
            WorkingTree::ChangesInProject
        };
        Ok(Self { repository, branch, working_tree, commits })
    }
}

impl GitCommit {
    fn parse_log(log: &str, user_email: &str, user_name: &str) -> Vec<Self> {
        log.split('\u{1e}')
            .filter(|entry| !entry.is_empty())
            .filter_map(|entry| {
                let entry = entry.trim_start();
                let mut fields = entry.split('\u{1f}');
                let sha = fields.next()?.get(..6)?.to_string();
                let author_name = fields.next()?;
                let author_email = fields.next()?;
                let authored_by_user = if user_email.is_empty() {
                    !user_name.is_empty() && author_name.eq_ignore_ascii_case(user_name)
                } else {
                    author_email.eq_ignore_ascii_case(user_email)
                };
                let author = initials(author_name);
                let message = fields.next()?.to_string();
                let age = fields.next()?.to_string();
                Some(Self { sha, author, authored_by_user, message, age })
            })
            .collect()
    }
}

fn git<const N: usize>(repository: &Path, args: [&str; N]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository)
        .output()
        .map_err(|error| format!("Could not run git: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn initials(author: &str) -> String {
    author
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::{GitCommit, initials};

    #[test]
    fn derives_author_initials() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("jan"), "J");
    }

    #[test]
    fn keeps_six_characters_for_each_commit_sha() {
        let log = concat!(
            "1234567890abcdef1234567890abcdef12345678\u{1f}Ada Lovelace\u{1f}ada@example.com\u{1f}First\u{1f}1 day ago\u{1e}",
            "\nabcdef1234567890abcdef1234567890abcdef12\u{1f}Grace Hopper\u{1f}grace@example.com\u{1f}Second\u{1f}2 days ago\u{1e}",
        );

        let commits = GitCommit::parse_log(log, "jan@example.com", "Jan Example");

        assert_eq!(commits.len(), 2);
        assert_eq!(
            commits.iter().map(|commit| commit.sha.as_str()).collect::<Vec<_>>(),
            ["123456", "abcdef"]
        );
        assert!(commits.iter().all(|commit| commit.sha.chars().count() == 6));
    }
}

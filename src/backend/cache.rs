//! Project snapshots read and written by background workers.
use super::Repository;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    home: std::path::PathBuf,
    skipped_dirs: Vec<String>,
    repositories: Vec<Repository>,
}

pub fn load(home: &Path, skipped_dirs: &[String]) -> Option<Vec<Repository>> {
    let bytes = fs::read(home.join(".cache/canopy/projects.json")).ok()?;
    let snapshot: Snapshot = serde_json::from_slice(&bytes).ok()?;
    if snapshot.version != 1 || snapshot.home != home || snapshot.skipped_dirs != skipped_dirs {
        return None;
    }
    if snapshot.repositories.iter().any(|repo| {
        !repo.path.starts_with(home)
            || repo.projects.is_empty()
            || repo
                .projects
                .iter()
                .any(|project| !project.path.starts_with(&repo.path) || project.kinds().is_empty())
    }) {
        return None;
    }
    Some(snapshot.repositories)
}

pub fn save(home: &Path, skipped_dirs: &[String], repositories: &[Repository]) -> io::Result<()> {
    let directory = home.join(".cache/canopy");
    fs::create_dir_all(&directory)?;
    let snapshot = Snapshot {
        version: 1,
        home: home.to_owned(),
        skipped_dirs: skipped_dirs.to_vec(),
        repositories: repositories.to_vec(),
    };
    let bytes = serde_json::to_vec(&snapshot)?;
    let temporary = directory.join(format!("projects.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)?;
    let result = fs::rename(&temporary, directory.join("projects.json"));
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Project, ProjectKind, commands::CommandKind};

    #[test]
    fn cache_roundtrip_and_invalid_snapshots() {
        let home = std::env::temp_dir().join(format!("canopy-cache-{}", std::process::id()));
        let skipped = vec!["target".to_owned()];
        assert!(load(&home, &skipped).is_none());
        let path = home.join("repo");
        let repositories = vec![Repository {
            path: path.clone(),
            projects: vec![Project::new(
                path.clone(),
                vec![ProjectKind::Rust],
                Some(vec!["test".into()]),
            )],
        }];
        save(&home, &skipped, &repositories).unwrap();
        let cached = load(&home, &skipped).unwrap();
        assert_eq!(cached[0].projects[0].path, path);
        assert_eq!(cached[0].projects[0].commands(CommandKind::Test), vec![vec!["just", "test"]]);
        assert!(load(&home, &[]).is_none());
        save(&home, &skipped, &[]).unwrap();
        assert!(load(&home, &skipped).unwrap().is_empty());
        fs::write(home.join(".cache/canopy/projects.json"), "{broken").unwrap();
        assert!(load(&home, &skipped).is_none());
        fs::remove_dir_all(home).unwrap();
    }
}

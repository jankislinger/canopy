//! Build identity and the cached check for newer commits on GitHub master.
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const REPOSITORY_URL: &str = "https://github.com/jankislinger/canopy.git";
pub const BRANCH: &str = "auto-upgrade";
const API_URL: &str = "https://api.github.com/repos/jankislinger/canopy";
const CACHE_LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);

/// The commit this binary was built from and where it runs from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildInfo {
    pub sha: String,
    pub dirty: bool,
    /// Runs from the cargo target directory it was built in, e.g. via `cargo run`.
    pub dev: bool,
    target_dir: PathBuf,
}

impl BuildInfo {
    pub fn current() -> Self {
        let target_dir = PathBuf::from(env!("CANOPY_TARGET_DIR"));
        let target_dir = target_dir.canonicalize().unwrap_or(target_dir);
        let dev = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.canonicalize().ok())
            .is_some_and(|exe| exe.starts_with(&target_dir));
        Self {
            sha: env!("CANOPY_BUILD_SHA").to_owned(),
            dirty: env!("CANOPY_BUILD_DIRTY") == "true",
            dev,
            target_dir,
        }
    }

    /// Formats the build for display, for example `Build ef12345`.
    pub fn label(&self) -> String {
        match (self.sha.get(..7), self.dirty) {
            (Some(short), true) => format!("Build {short}-dirty"),
            (Some(short), false) => format!("Build {short}"),
            (None, _) => "Build unknown".into(),
        }
    }

    /// Dev builds keep their own cache so they never replace the installed binary's result.
    fn cache_path(&self, home: &Path) -> PathBuf {
        if self.dev {
            self.target_dir.join("canopy-update.json")
        } else {
            home.join(".cache/canopy/update.json")
        }
    }
}

/// Master has commits this build does not contain.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    pub latest: String,
    pub ahead_by: u64,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    build: String,
    checked_at: u64,
    latest: String,
    ahead_by: u64,
}

fn load(path: &Path, build: &str, now: u64) -> Option<Snapshot> {
    let snapshot: Snapshot = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let fresh = snapshot.checked_at <= now && now - snapshot.checked_at < CACHE_LIFETIME.as_secs();
    (snapshot.version == 1 && snapshot.build == build && fresh).then_some(snapshot)
}

fn save(path: &Path, snapshot: &Snapshot) -> io::Result<()> {
    let directory = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!("update.{}.tmp", std::process::id()));
    fs::write(&temporary, serde_json::to_vec(snapshot)?)?;
    let result = fs::rename(&temporary, path);
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

/// Returns the update available for this build, using the cache when it is fresh.
///
/// Network failures return `None` and are not cached.
pub fn check(home: &Path, build: &BuildInfo) -> Option<UpdateInfo> {
    if build.sha.is_empty() {
        return None;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let path = build.cache_path(home);
    let snapshot = match load(&path, &build.sha, now) {
        Some(snapshot) => snapshot,
        None => {
            let (latest, ahead_by) = fetch(&build.sha)?;
            let snapshot = Snapshot {
                version: 1,
                build: build.sha.clone(),
                checked_at: now,
                latest,
                ahead_by,
            };
            let _ = save(&path, &snapshot);
            snapshot
        }
    };
    (snapshot.ahead_by > 0)
        .then_some(UpdateInfo { latest: snapshot.latest, ahead_by: snapshot.ahead_by })
}

/// Fetches the master sha and how many of its commits `build` lacks.
fn fetch(build: &str) -> Option<(String, u64)> {
    let (status, latest) =
        get(&format!("{API_URL}/commits/{BRANCH}"), "application/vnd.github.sha")?;
    let latest = latest.trim().to_owned();
    if status != 200 || latest.is_empty() {
        return None;
    }
    if latest == build {
        return Some((latest, 0));
    }
    let (status, body) =
        get(&format!("{API_URL}/compare/{build}...{latest}"), "application/vnd.github+json")?;
    match status {
        200 => Some((latest, parse_compare(&body)?)),
        // GitHub does not know commits that were never pushed.
        404 => Some((latest, 0)),
        _ => None,
    }
}

/// Counts master's new commits; a build that is identical to or ahead of master has none.
fn parse_compare(body: &str) -> Option<u64> {
    #[derive(Deserialize)]
    struct Compare {
        status: String,
        ahead_by: u64,
    }
    let compare: Compare = serde_json::from_str(body).ok()?;
    Some(match compare.status.as_str() {
        "ahead" | "diverged" => compare.ahead_by,
        _ => 0,
    })
}

fn get(url: &str, accept: &str) -> Option<(u16, String)> {
    let output = Command::new("curl")
        .args(["-sS", "--max-time", "5", "-w", "\n%{http_code}"])
        .args(["-H", &format!("Accept: {accept}"), "-H", "User-Agent: canopy", url])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, status) = text.rsplit_once('\n')?;
    Some((status.trim().parse().ok()?, body.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(build: &str, checked_at: u64) -> Snapshot {
        Snapshot { version: 1, build: build.into(), checked_at, latest: "bbb".into(), ahead_by: 2 }
    }

    #[test]
    fn cache_is_fresh_for_eight_hours_and_tied_to_the_build() {
        let directory = std::env::temp_dir().join(format!("canopy-update-{}", std::process::id()));
        let path = directory.join("update.json");
        assert!(load(&path, "aaa", 100).is_none());
        save(&path, &snapshot("aaa", 100)).unwrap();
        assert_eq!(load(&path, "aaa", 100).unwrap().latest, "bbb");
        assert!(load(&path, "aaa", 100 + CACHE_LIFETIME.as_secs() - 1).is_some());
        assert!(load(&path, "aaa", 100 + CACHE_LIFETIME.as_secs()).is_none());
        assert!(load(&path, "aaa", 99).is_none());
        assert!(load(&path, "ccc", 100).is_none());
        fs::write(&path, "{broken").unwrap();
        assert!(load(&path, "aaa", 100).is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn only_missing_master_commits_count_as_updates() {
        let body = |status: &str| format!(r#"{{"status":"{status}","ahead_by":3,"behind_by":1}}"#);
        assert_eq!(parse_compare(&body("ahead")), Some(3));
        assert_eq!(parse_compare(&body("diverged")), Some(3));
        assert_eq!(parse_compare(&body("behind")), Some(0));
        assert_eq!(parse_compare(&body("identical")), Some(0));
        assert_eq!(parse_compare("not json"), None);
    }

    #[test]
    fn labels_use_the_short_sha() {
        let build = |sha: &str, dirty| BuildInfo {
            sha: sha.into(),
            dirty,
            dev: false,
            target_dir: PathBuf::new(),
        };
        assert_eq!(build("ef12345678", false).label(), "Build ef12345");
        assert_eq!(build("ef12345678", true).label(), "Build ef12345-dirty");
        assert_eq!(build("", false).label(), "Build unknown");
    }
}

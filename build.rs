use std::{path::PathBuf, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|status| !status.is_empty());
    println!("cargo:rustc-env=CANOPY_BUILD_SHA={sha}");
    println!("cargo:rustc-env=CANOPY_BUILD_DIRTY={dirty}");

    // OUT_DIR is target/<profile>/build/canopy-<hash>/out.
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let target_dir = out_dir.ancestors().nth(3).unwrap_or(&out_dir);
    println!("cargo:rustc-env=CANOPY_TARGET_DIR={}", target_dir.display());

    if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        for path in ["HEAD", "index", "refs", "packed-refs"] {
            println!("cargo:rerun-if-changed={git_dir}/{path}");
        }
    }
    // Tracked edits change the dirty flag without touching git metadata.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");
}

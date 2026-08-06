//! Embeds the git commit the binary was built from into `CASEGRAPHEN_GIT_DESCRIBE`,
//! read back by `src/cli.rs`'s `version` command via `option_env!`.
//!
//! Degrades to nothing (the plain `CARGO_PKG_VERSION` is shown instead) when
//! `git` is unavailable or `CARGO_MANIFEST_DIR` is not inside a git repository
//! — which is exactly the case for a crate unpacked from the registry, so
//! `cargo package`/`cargo install` from crates.io keep working with no error.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();

    if let Some(git_dir) = git_dir(&manifest_dir) {
        // Track the files that change when the checked-out commit or the
        // working tree's dirty state changes, so `--version` never goes
        // stale without a source edit to force a rebuild.
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
        println!("cargo:rerun-if-changed={}", git_dir.join("index").display());
    }

    if let Some(describe) = git_describe(&manifest_dir) {
        println!("cargo:rustc-env=CASEGRAPHEN_GIT_DESCRIBE={describe}");
    }
}

fn git_describe(manifest_dir: &str) -> Option<String> {
    let sha = run_git(manifest_dir, &["rev-parse", "--short", "HEAD"])?;
    let dirty = !run_git(manifest_dir, &["status", "--porcelain"])?.is_empty();
    Some(if dirty { format!("{sha}-dirty") } else { sha })
}

/// Resolves the real git directory for `manifest_dir`, following a linked
/// worktree's `.git` file (as opposed to assuming `.git` is always a
/// directory) so `HEAD` is the worktree's own, not the main checkout's.
fn git_dir(manifest_dir: &str) -> Option<PathBuf> {
    let raw = run_git(manifest_dir, &["rev-parse", "--git-dir"])?;
    let path = Path::new(&raw);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(manifest_dir).join(path)
    })
}

fn run_git(manifest_dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest_dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

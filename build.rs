//! Embeds the git commit the binary was built from into `CASEGRAPHEN_GIT_DESCRIBE`,
//! read back by `src/cli.rs`'s `version` command via `option_env!`.
//!
//! Degrades to nothing (the plain `CARGO_PKG_VERSION` is shown instead) when
//! `git` is unavailable or `CARGO_MANIFEST_DIR` is not inside a git repository
//! — which is exactly the case for a crate unpacked from the registry, so
//! `cargo package`/`cargo install` from crates.io keep working with no error.
//!
//! ## What "dirty" means
//!
//! Dirty means *tracked* files differ from `HEAD` (`git status --porcelain
//! --untracked-files=no`). An untracked file is not part of this identifier's
//! claim: every `include_str!`/`include_bytes!` in this crate names a path
//! under `schemas/`, `src/`, or `tests/fixtures/` that is itself committed
//! (audited by grep over the crate), and a fresh checkout or `cargo package`
//! tarball only contains tracked files anyway — so a file that could
//! legitimately change this build but is untracked would already fail to
//! build outside this working tree, which is a defect in that file's
//! tracking, not something this marker should paper over by flagging every
//! stray scratch file as dirty. `scripts/resource-allocator-release-scale-pilot.py`
//! already uses the same `--untracked-files=no` definition for the same reason.
//!
//! ## Why every tracked file is watched
//!
//! Measured before choosing this: forcing `build.rs` to rerun unconditionally
//! (dropping scoped `rerun-if-changed` in favor of a path that never exists)
//! turned a no-op `cargo build` from ~0.03s into ~2.2s on this crate, because
//! cargo cannot tell the rerun produced the same output ahead of time — it
//! recompiles the lib and all three binaries every time regardless of whether
//! the printed value actually changed. That cost is paid on every build,
//! including ones where nothing about the identifier could have changed, so
//! it is not the right trade.
//!
//! Scoping to `HEAD` and `index` alone (the previous behavior) under-reports:
//! `HEAD` is a symbolic ref (`ref: refs/heads/<branch>`) whose *content*
//! doesn't change when a commit lands on the same branch — only the resolved
//! ref file does — so a rebase or amend leaves the embedded commit stale
//! (observed in #120). And editing a tracked file makes the tree dirty
//! without touching `HEAD` or the index at all, so an uncommitted edit leaves
//! the embedded `-dirty` suffix stale too (observed in #124). Both are fixed
//! by watching exactly the paths whose content decides `git_describe`'s
//! answer: `HEAD` and `index` (worktree-private — see `git_dir`), the
//! resolved branch ref and `packed-refs` (shared across a linked worktree's
//! checkouts, so resolved against `git_common_dir`, not `git_dir` — refs
//! live in the common dir even when `HEAD` doesn't), and every path
//! `git ls-files` reports as tracked. `git ls-files` on this crate costs
//! ~10ms and adds under a thousand extra paths for cargo to stat —
//! negligible next to the 2.2s unconditional-rerun cost, and it changes
//! nothing about the common case (editing a tracked source file was already
//! going to trigger cargo's own recompilation of that file; this only makes
//! the build script rerun alongside it instead of lagging behind).

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();

    if let Some(git_dir) = resolved_dir(&manifest_dir, "--git-dir") {
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
        println!("cargo:rerun-if-changed={}", git_dir.join("index").display());
    }

    if let Some(common_dir) = resolved_dir(&manifest_dir, "--git-common-dir") {
        // Cargo treats a watched path that doesn't exist as always-changed
        // (that is how it is told to force a rerun), so only watch these if
        // they are actually present: most repositories never pack refs, and
        // a repository with no `packed-refs` would otherwise force build.rs
        // to rerun on every single build forever, not just when it matters.
        let packed_refs = common_dir.join("packed-refs");
        if packed_refs.exists() {
            println!("cargo:rerun-if-changed={}", packed_refs.display());
        }
        if let Some(resolved) = run_git(&manifest_dir, &["symbolic-ref", "-q", "HEAD"]) {
            let ref_file = common_dir.join(resolved);
            if ref_file.exists() {
                println!("cargo:rerun-if-changed={}", ref_file.display());
            }
        }
    }

    if let Some(files) = run_git(&manifest_dir, &["ls-files"]) {
        for file in files.lines() {
            // Same existence guard as above, and for the same reason: `git
            // ls-files` lists what the index records, which still names a
            // file deleted from the worktree without `git rm`/`git add`. A
            // missing watched path forces every subsequent build to rerun
            // build.rs and recompile, not just the one that should react to
            // it — measured at the same ~2.2s this file's watching was
            // designed to avoid paying unconditionally.
            //
            // Known blind spot from this guard: deleting a tracked file
            // without staging the deletion makes `git status` (and this
            // marker's `-dirty`) correctly report dirty, but nothing here
            // watches a path that isn't there to notice the deletion
            // happened, so `--version` can under-report until some other
            // tracked file changes, HEAD moves, or the deletion is staged
            // (which touches `index`, which is always watched). This is the
            // deliberate price of not paying the unconditional-rerun cost
            // for it: watching the missing path anyway reproduces that
            // 2.2s-per-build cost for as long as the file stays deleted, and
            // watching each tracked file's parent directory (which would
            // catch a deletion via the directory's own mtime) is a heavier,
            // different design — more paths, and cargo's own docs warn
            // directory watches don't scale to large trees — for a case
            // that is rare (an uncommitted deletion, as opposed to an edit)
            // and already visible plainly in `git status`. Not closed here.
            let tracked = Path::new(&manifest_dir).join(file);
            if tracked.exists() {
                println!("cargo:rerun-if-changed={}", tracked.display());
            }
        }
    }

    if let Some(describe) = git_describe(&manifest_dir) {
        println!("cargo:rustc-env=CASEGRAPHEN_GIT_DESCRIBE={describe}");
    }
}

fn git_describe(manifest_dir: &str) -> Option<String> {
    let sha = run_git(manifest_dir, &["rev-parse", "--short", "HEAD"])?;
    let dirty = !run_git(
        manifest_dir,
        &["status", "--porcelain", "--untracked-files=no"],
    )?
    .is_empty();
    Some(if dirty { format!("{sha}-dirty") } else { sha })
}

/// Resolves a git directory for `manifest_dir` via the given `rev-parse`
/// flag, following a linked worktree's `.git` file (as opposed to assuming
/// `.git` is always a directory in place). `--git-dir` gives the
/// worktree-private dir (`HEAD`, `index`); `--git-common-dir` gives the dir
/// shared across all of a repository's worktrees (`refs/`, `packed-refs`).
/// They differ for a linked worktree and must not be used interchangeably.
fn resolved_dir(manifest_dir: &str, which: &str) -> Option<PathBuf> {
    let raw = run_git(manifest_dir, &["rev-parse", which])?;
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

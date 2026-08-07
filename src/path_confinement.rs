//! The single canonicalized-root path-containment predicate.
//!
//! Every command that resolves a caller-supplied relative path against a
//! confinement directory — `packet apply`'s `artifacts:` entries
//! (`native_cli/ops/mutations.rs`), the GitHub evidence adapter's
//! `capture_manifest.v0` `artifact_path` entries (`github_evidence/normalize.rs`)
//! — checks the result with this and only this predicate. CLAUDE.md: a
//! decision rule has exactly one implementation; before this module existed
//! the check was defined once in `native_cli/ops/mutations.rs` and would have
//! been copied for the second caller, which is exactly the drift this module
//! closes off.
//!
//! This predicate is deliberately narrow — it does not itself canonicalize
//! anything or reject `..`/absolute entries. Callers still own their own
//! three-stage confined resolution (lexical rejection of the raw entry,
//! canonicalize the entry joined onto the root, then this containment check)
//! because stage 1 depends on caller-specific refusal reporting; only the
//! containment check itself (stage 3) is the shared rule.

use std::path::Path;

/// Whether a canonicalized path stays inside a canonicalized confinement
/// root. Both arguments must already be canonicalized by the caller — this
/// predicate does not resolve symlinks itself, so it is safe to use only as
/// the final stage of a caller's own confined resolution: it is what catches
/// an in-tree symlink whose target leaves the root, which a lexical check on
/// the raw entry cannot see. A plain string-prefix check would wrongly treat
/// a sibling directory whose name extends the root's as contained (e.g.
/// `/tmp/rootless` against root `/tmp/root`); `Path::starts_with` compares
/// components, not bytes, so it does not make that mistake.
pub(crate) fn path_confined(canonical_path: &Path, canonical_root: &Path) -> bool {
    canonical_path.starts_with(canonical_root)
}

#[cfg(test)]
mod tests {
    use super::path_confined;
    use std::path::Path;

    #[test]
    fn checks_a_canonical_component_prefix_not_a_byte_prefix() {
        let root = Path::new("/tmp/root");
        assert!(path_confined(Path::new("/tmp/root/file.txt"), root));
        assert!(path_confined(root, root));
        // A string-prefix check would wrongly accept this: "/tmp/rootless"
        // starts with the bytes "/tmp/root" but is a sibling, not a child.
        assert!(!path_confined(Path::new("/tmp/rootless/file.txt"), root));
        assert!(!path_confined(Path::new("/tmp/other/file.txt"), root));
    }
}

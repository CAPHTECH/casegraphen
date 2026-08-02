//! Explicit adapter for isolated Git worktrees.
//!
//! The adapter operates only on caller-supplied repository and worktree paths.
//! It never selects a base revision, infers liveness from elapsed time, or
//! cleans a worktree without a matching disposition assertion.

use crate::resource_protocol::{
    validate_worktree_record, GitWorktreeRecord, ReservationAssertionKind,
    ReservationDispositionAssertion, ResourceReservation, WorktreeCleanupPolicy, WorktreeState,
    RESERVATION_ASSERTION_SCHEMA, WORKTREE_RECORD_SCHEMA,
};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

/// Inputs that bind one reservation/attempt to one explicitly located Git
/// worktree. `allowed_write_paths` are repository-relative path prefixes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitWorktreeRequest {
    pub repository_path: PathBuf,
    pub worktree_path: PathBuf,
    pub worktree_id: String,
    pub reservation_id: String,
    pub attempt_id: String,
    pub path_identity: String,
    pub base_commit_sha: String,
    pub branch: String,
    pub allowed_write_paths: Vec<String>,
}

/// A located adapter refusal. Command stderr is retained for diagnosis but is
/// never interpreted as an accepted resource fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeAdapterError {
    pub code: String,
    pub detail: String,
}

impl fmt::Display for WorktreeAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for WorktreeAdapterError {}

/// Creates an isolated worktree and new branch from the exact requested base
/// commit. The destination must not exist and its parent must already exist.
pub fn create_isolated_worktree(
    request: &GitWorktreeRequest,
    reservation: &ResourceReservation,
) -> Result<GitWorktreeRecord, WorktreeAdapterError> {
    validate_request(request, reservation, true)?;
    let repository = canonical_repository(&request.repository_path)?;
    let parent = canonical_parent(&request.worktree_path)?;
    if request.worktree_path.exists() {
        return Err(error(
            "worktree_path_exists",
            "worktree destination must not already exist",
        ));
    }
    if request.worktree_path.starts_with(&repository) {
        return Err(error(
            "worktree_inside_repository",
            "isolated worktree destination must be outside the source repository",
        ));
    }
    let destination = parent.join(request.worktree_path.file_name().ok_or_else(|| {
        error(
            "invalid_worktree_path",
            "destination has no final component",
        )
    })?);
    validate_base_and_branch(&repository, request)?;
    run_git(
        &repository,
        [
            "worktree",
            "add",
            "-b",
            request.branch.as_str(),
            destination
                .to_str()
                .ok_or_else(|| error("non_utf8_worktree_path", "worktree path must be UTF-8"))?,
            request.base_commit_sha.as_str(),
        ],
        "git_worktree_add_failed",
    )?;

    Ok(record(
        request,
        None,
        true,
        Vec::new(),
        WorktreeState::Active,
    ))
}

/// Observes the actual HEAD, dirty state, and all paths changed since the base.
/// A clean worktree may still contain unexpected committed writes, so both the
/// base-to-HEAD diff and the working-tree status are inspected.
pub fn observe_isolated_worktree(
    request: &GitWorktreeRequest,
    reservation: &ResourceReservation,
) -> Result<GitWorktreeRecord, WorktreeAdapterError> {
    validate_request(request, reservation, false)?;
    let repository = canonical_repository(&request.repository_path)?;
    let worktree = canonical_worktree(&request.worktree_path)?;
    assert_registered_worktree(&repository, &worktree)?;

    let head = git_stdout(
        &worktree,
        ["rev-parse", "--verify", "HEAD"],
        "git_head_failed",
    )?;
    if !is_git_sha(&head) {
        return Err(error(
            "invalid_observed_head",
            "Git returned a non-canonical HEAD",
        ));
    }
    let dirty_paths = nul_paths(run_git(
        &worktree,
        ["diff", "--name-only", "-z"],
        "git_dirty_diff_failed",
    )?);
    let staged_paths = nul_paths(run_git(
        &worktree,
        ["diff", "--cached", "--name-only", "-z"],
        "git_staged_diff_failed",
    )?);
    let untracked_paths = nul_paths(run_git(
        &worktree,
        ["ls-files", "--others", "--exclude-standard", "-z"],
        "git_untracked_list_failed",
    )?);
    let committed_paths = nul_paths(run_git(
        &worktree,
        [
            "diff",
            "--name-only",
            "-z",
            request.base_commit_sha.as_str(),
            head.as_str(),
        ],
        "git_base_diff_failed",
    )?);
    let dirty = !dirty_paths.is_empty() || !staged_paths.is_empty() || !untracked_paths.is_empty();
    let changed_paths = dirty_paths
        .into_iter()
        .chain(staged_paths)
        .chain(untracked_paths)
        .chain(committed_paths)
        .collect::<BTreeSet<_>>();
    let unexpected_write_paths = changed_paths
        .into_iter()
        .filter(|path| !path_is_allowed(path, &request.allowed_write_paths))
        .collect();
    Ok(record(
        request,
        Some(head),
        !dirty,
        unexpected_write_paths,
        if dirty {
            WorktreeState::Active
        } else {
            WorktreeState::Committed
        },
    ))
}

/// Removes a clean, committed worktree only after a matching explicit release
/// or supersede assertion. The branch and commit remain in the repository, so
/// the operation is recoverable through the recorded identities.
pub fn dispose_isolated_worktree(
    request: &GitWorktreeRequest,
    reservation: &ResourceReservation,
    assertion: &ReservationDispositionAssertion,
) -> Result<GitWorktreeRecord, WorktreeAdapterError> {
    validate_assertion(request, assertion)?;
    let mut observed = observe_isolated_worktree(request, reservation)?;
    if !observed.working_tree_clean {
        return Err(error(
            "dirty_worktree_cleanup_refused",
            "commit or recover dirty writes before cleanup",
        ));
    }
    if !observed.unexpected_write_paths.is_empty() {
        return Err(error(
            "unexpected_write_cleanup_refused",
            "unexpected writes require independent resolution before cleanup",
        ));
    }
    let repository = canonical_repository(&request.repository_path)?;
    let worktree = canonical_worktree(&request.worktree_path)?;
    run_git(
        &repository,
        [
            "worktree",
            "remove",
            worktree
                .to_str()
                .ok_or_else(|| error("non_utf8_worktree_path", "worktree path must be UTF-8"))?,
        ],
        "git_worktree_remove_failed",
    )?;
    observed.state = match assertion.kind {
        ReservationAssertionKind::Release => WorktreeState::CleanupApproved,
        ReservationAssertionKind::Supersede => WorktreeState::Superseded,
    };
    Ok(observed)
}

fn validate_request(
    request: &GitWorktreeRequest,
    reservation: &ResourceReservation,
    creation: bool,
) -> Result<(), WorktreeAdapterError> {
    if request.reservation_id != reservation.reservation_id
        || request.attempt_id != reservation.attempt_id
    {
        return Err(error(
            "worktree_join_mismatch",
            "request must join the exact reservation and attempt",
        ));
    }
    if !is_git_sha(&request.base_commit_sha) {
        return Err(error(
            "invalid_base_commit_sha",
            "base commit must be an explicit lowercase 40-character Git SHA",
        ));
    }
    if request.repository_path.as_os_str().is_empty()
        || request.worktree_path.as_os_str().is_empty()
        || !request.repository_path.is_absolute()
        || !request.worktree_path.is_absolute()
        || request.worktree_path == Path::new("/")
    {
        return Err(error(
            "unsafe_path",
            "repository and worktree paths must be explicit absolute paths",
        ));
    }
    for allowed in &request.allowed_write_paths {
        validate_relative_path(allowed)?;
    }
    let provisional = record(
        request,
        if creation {
            None
        } else {
            Some(request.base_commit_sha.clone())
        },
        true,
        Vec::new(),
        WorktreeState::Active,
    );
    let invalid = validate_worktree_record(&provisional, reservation)
        .into_iter()
        .filter(|finding| finding.code != "worktree_uncommitted")
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    if !invalid.is_empty() {
        return Err(error(
            "invalid_worktree_contract",
            format!("resource contract findings: {}", invalid.join(", ")),
        ));
    }
    Ok(())
}

fn validate_base_and_branch(
    repository: &Path,
    request: &GitWorktreeRequest,
) -> Result<(), WorktreeAdapterError> {
    let verified = git_stdout(
        repository,
        [
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", request.base_commit_sha),
        ],
        "unknown_base_commit",
    )?;
    if verified != request.base_commit_sha {
        return Err(error(
            "base_commit_mismatch",
            "the requested base must resolve to the exact supplied commit",
        ));
    }
    run_git(
        repository,
        ["check-ref-format", "--branch", request.branch.as_str()],
        "invalid_worktree_branch",
    )?;
    let existing = Command::new("git")
        .current_dir(repository)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", request.branch),
        ])
        .status()
        .map_err(|source| error("git_spawn_failed", source.to_string()))?;
    if existing.success() {
        return Err(error(
            "worktree_branch_exists",
            "adapter only creates a new branch for an isolated attempt",
        ));
    }
    Ok(())
}

fn validate_assertion(
    request: &GitWorktreeRequest,
    assertion: &ReservationDispositionAssertion,
) -> Result<(), WorktreeAdapterError> {
    let valid = assertion.schema == RESERVATION_ASSERTION_SCHEMA
        && assertion.schema_version == 0
        && assertion.reservation_id == request.reservation_id
        && assertion.attempt_id == request.attempt_id
        && !assertion.assertion_id.is_empty()
        && !assertion.asserted_by.is_empty()
        && !assertion.reason.is_empty()
        && match assertion.kind {
            ReservationAssertionKind::Release => assertion.superseding_reservation_id.is_none(),
            ReservationAssertionKind::Supersede => assertion
                .superseding_reservation_id
                .as_deref()
                .is_some_and(|id| !id.is_empty() && id != request.reservation_id),
        };
    if !valid {
        return Err(error(
            "invalid_cleanup_assertion",
            "cleanup requires an exact explicit release or supersede assertion",
        ));
    }
    Ok(())
}

fn record(
    request: &GitWorktreeRequest,
    resulting_commit_sha: Option<String>,
    working_tree_clean: bool,
    unexpected_write_paths: Vec<String>,
    state: WorktreeState,
) -> GitWorktreeRecord {
    GitWorktreeRecord {
        schema: WORKTREE_RECORD_SCHEMA.to_owned(),
        schema_version: 0,
        worktree_id: request.worktree_id.clone(),
        reservation_id: request.reservation_id.clone(),
        attempt_id: request.attempt_id.clone(),
        path_identity: request.path_identity.clone(),
        base_commit_sha: request.base_commit_sha.clone(),
        branch: request.branch.clone(),
        resulting_commit_sha,
        working_tree_clean,
        unexpected_write_paths,
        state,
        cleanup: WorktreeCleanupPolicy {
            method: "git-branch-retained".to_owned(),
            recoverable: true,
            requires_explicit_assertion: true,
        },
    }
}

fn canonical_repository(path: &Path) -> Result<PathBuf, WorktreeAdapterError> {
    let canonical = fs::canonicalize(path)
        .map_err(|source| error("invalid_repository_path", source.to_string()))?;
    let inside = git_stdout(
        &canonical,
        ["rev-parse", "--show-toplevel"],
        "not_git_repository",
    )?;
    let top = fs::canonicalize(inside)
        .map_err(|source| error("invalid_repository_path", source.to_string()))?;
    if top != canonical {
        return Err(error(
            "repository_path_not_root",
            "repository path must name the exact worktree root",
        ));
    }
    Ok(canonical)
}

fn canonical_parent(path: &Path) -> Result<PathBuf, WorktreeAdapterError> {
    let parent = path
        .parent()
        .ok_or_else(|| error("invalid_worktree_path", "destination has no parent"))?;
    fs::canonicalize(parent).map_err(|source| error("invalid_worktree_parent", source.to_string()))
}

fn canonical_worktree(path: &Path) -> Result<PathBuf, WorktreeAdapterError> {
    fs::canonicalize(path).map_err(|source| error("missing_worktree_path", source.to_string()))
}

fn assert_registered_worktree(
    repository: &Path,
    worktree: &Path,
) -> Result<(), WorktreeAdapterError> {
    let common = git_stdout(
        worktree,
        ["rev-parse", "--git-common-dir"],
        "not_git_worktree",
    )?;
    let repository_git = fs::canonicalize(repository.join(".git"))
        .map_err(|source| error("invalid_repository_git_dir", source.to_string()))?;
    let common_path = Path::new(&common);
    let resolved = if common_path.is_absolute() {
        fs::canonicalize(common_path)
    } else {
        fs::canonicalize(worktree.join(common_path))
    }
    .map_err(|source| error("invalid_worktree_common_dir", source.to_string()))?;
    if resolved != repository_git {
        return Err(error(
            "foreign_worktree",
            "worktree does not belong to the requested repository",
        ));
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), WorktreeAdapterError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(error(
            "invalid_allowed_write_path",
            "allowed write paths must be normalized repository-relative paths",
        ));
    }
    Ok(())
}

fn path_is_allowed(path: &str, allowed: &[String]) -> bool {
    let candidate = Path::new(path);
    allowed.iter().any(|prefix| candidate.starts_with(prefix))
}

fn nul_paths(output: Output) -> BTreeSet<String> {
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .collect()
}

fn git_stdout<const N: usize>(
    directory: &Path,
    args: [&str; N],
    code: &str,
) -> Result<String, WorktreeAdapterError> {
    let output = run_git(directory, args, code)?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|source| error("non_utf8_git_output", source.to_string()))
}

fn run_git<const N: usize>(
    directory: &Path,
    args: [&str; N],
    code: &str,
) -> Result<Output, WorktreeAdapterError> {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .map_err(|source| error("git_spawn_failed", source.to_string()))?;
    if !output.status.success() {
        return Err(error(code, String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(output)
}

fn is_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn error(code: &str, detail: impl Into<String>) -> WorktreeAdapterError {
    WorktreeAdapterError {
        code: code.to_owned(),
        detail: detail.into(),
    }
}

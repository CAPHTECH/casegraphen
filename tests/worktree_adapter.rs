#![allow(missing_docs)]

use casegraphen::execution_topology::{ResourceMode, WorkspaceStrategy};
use casegraphen::resource_protocol::{
    declaration_grants, ReservationAssertionKind, ReservationDispositionAssertion,
    ResourceDeclaration, ResourceReservation, WorktreeState, RESERVATION_ASSERTION_SCHEMA,
    RESOURCE_DECLARATION_SCHEMA, RESOURCE_RESERVATION_SCHEMA,
};
use casegraphen::worktree_adapter::{
    create_isolated_worktree, dispose_isolated_worktree, observe_isolated_worktree,
    GitWorktreeRequest,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct DisposableRepository {
    root: PathBuf,
    repository: PathBuf,
}

impl DisposableRepository {
    fn new() -> Self {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "casegraphen-worktree-adapter-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create disposable root");
        let repository = root.join("repository");
        fs::create_dir(&repository).expect("create repository");
        git(&repository, &["init", "--initial-branch=main"]);
        git(
            &repository,
            &["config", "user.email", "fixture@example.invalid"],
        );
        git(&repository, &["config", "user.name", "CaseGraphen Fixture"]);
        fs::write(repository.join("base.txt"), "base\n").expect("write base");
        git(&repository, &["add", "base.txt"]);
        git(&repository, &["commit", "-m", "base"]);
        Self { root, repository }
    }

    fn base_commit(&self) -> String {
        git_stdout(&self.repository, &["rev-parse", "HEAD"])
    }

    fn worktree(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for DisposableRepository {
    fn drop(&mut self) {
        assert!(self.root.starts_with(std::env::temp_dir()));
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn two_nodes_create_distinct_worktrees_and_commits_then_explicitly_release() {
    let fixture = DisposableRepository::new();
    let base = fixture.base_commit();
    let mut observed = Vec::new();

    for suffix in ["a", "b"] {
        let request = request(&fixture, &base, suffix, vec![format!("node-{suffix}.txt")]);
        let reservation = reservation(suffix);
        let active = create_isolated_worktree(&request, &reservation).expect("create worktree");
        assert_eq!(active.state, WorktreeState::Active);
        assert_eq!(active.base_commit_sha, base);

        fs::write(
            request.worktree_path.join(format!("node-{suffix}.txt")),
            format!("node {suffix}\n"),
        )
        .expect("write node output");
        git(&request.worktree_path, &["add", "."]);
        git(
            &request.worktree_path,
            &["commit", "-m", &format!("node {suffix}")],
        );
        let record = observe_isolated_worktree(&request, &reservation).expect("observe worktree");
        assert_eq!(record.state, WorktreeState::Committed);
        assert!(record.working_tree_clean);
        assert!(record.unexpected_write_paths.is_empty());
        assert_ne!(record.resulting_commit_sha.as_deref(), Some(base.as_str()));

        let disposed =
            dispose_isolated_worktree(&request, &reservation, &release_assertion(suffix))
                .expect("explicit recoverable cleanup");
        assert_eq!(disposed.state, WorktreeState::CleanupApproved);
        assert!(!request.worktree_path.exists());
        observed.push((record.path_identity, record.resulting_commit_sha.unwrap()));
    }

    assert_ne!(observed[0].0, observed[1].0);
    assert_ne!(observed[0].1, observed[1].1);
    assert_eq!(
        git_stdout(&fixture.repository, &["branch", "--list", "node-a"]),
        "node-a"
    );
    assert_eq!(
        git_stdout(&fixture.repository, &["branch", "--list", "node-b"]),
        "node-b"
    );
}

#[test]
fn dirty_or_unexpected_writes_are_observed_and_cleanup_is_refused() {
    let fixture = DisposableRepository::new();
    let base = fixture.base_commit();
    let request = request(&fixture, &base, "dirty", vec!["allowed.txt".to_owned()]);
    let reservation = reservation("dirty");
    create_isolated_worktree(&request, &reservation).expect("create worktree");
    fs::write(request.worktree_path.join("outside.txt"), "unexpected\n").expect("write dirty");

    let record = observe_isolated_worktree(&request, &reservation).expect("observe dirty");
    assert!(!record.working_tree_clean);
    assert_eq!(record.unexpected_write_paths, ["outside.txt"]);
    let refusal = dispose_isolated_worktree(&request, &reservation, &release_assertion("dirty"))
        .expect_err("dirty worktree must be retained");
    assert_eq!(refusal.code, "dirty_worktree_cleanup_refused");
    assert!(request.worktree_path.exists());

    git(&request.worktree_path, &["add", "outside.txt"]);
    git(&request.worktree_path, &["commit", "-m", "unexpected path"]);
    let record = observe_isolated_worktree(&request, &reservation).expect("observe committed path");
    assert!(record.working_tree_clean);
    assert_eq!(record.unexpected_write_paths, ["outside.txt"]);
    let refusal = dispose_isolated_worktree(&request, &reservation, &release_assertion("dirty"))
        .expect_err("unexpected committed write must be retained");
    assert_eq!(refusal.code, "unexpected_write_cleanup_refused");
}

#[test]
fn creation_requires_the_exact_explicit_base_and_reserved_identity() {
    let fixture = DisposableRepository::new();
    let base = fixture.base_commit();
    let mut request = request(&fixture, &base, "invalid", vec!["allowed.txt".to_owned()]);
    let reservation = reservation("invalid");
    request.base_commit_sha = "0".repeat(40);
    let refusal = create_isolated_worktree(&request, &reservation).expect_err("unknown base");
    assert_eq!(refusal.code, "unknown_base_commit");

    request.base_commit_sha = base;
    request.attempt_id = "attempt:other".to_owned();
    let refusal = create_isolated_worktree(&request, &reservation).expect_err("wrong join");
    assert_eq!(refusal.code, "worktree_join_mismatch");
}

#[test]
fn cleanup_never_uses_time_or_a_mismatched_assertion() {
    let fixture = DisposableRepository::new();
    let base = fixture.base_commit();
    let request = request(&fixture, &base, "assertion", vec!["allowed.txt".to_owned()]);
    let reservation = reservation("assertion");
    create_isolated_worktree(&request, &reservation).expect("create worktree");
    let mut assertion = release_assertion("assertion");
    assertion.attempt_id = "attempt:other".to_owned();
    let refusal = dispose_isolated_worktree(&request, &reservation, &assertion)
        .expect_err("mismatched assertion cannot cleanup");
    assert_eq!(refusal.code, "invalid_cleanup_assertion");
    assert!(request.worktree_path.exists());

    let supersede = ReservationDispositionAssertion {
        schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
        schema_version: 0,
        assertion_id: "assertion:supersede-assertion".to_owned(),
        reservation_id: "reservation:assertion".to_owned(),
        attempt_id: "attempt:assertion".to_owned(),
        kind: ReservationAssertionKind::Supersede,
        asserted_by: "operator:fixture".to_owned(),
        reason: "a replacement reservation was explicitly authorized".to_owned(),
        superseding_reservation_id: Some("reservation:replacement".to_owned()),
    };
    let disposed = dispose_isolated_worktree(&request, &reservation, &supersede)
        .expect("explicit supersede cleans recoverably");
    assert_eq!(disposed.state, WorktreeState::Superseded);
    assert!(!request.worktree_path.exists());
}

fn request(
    fixture: &DisposableRepository,
    base: &str,
    suffix: &str,
    allowed_write_paths: Vec<String>,
) -> GitWorktreeRequest {
    GitWorktreeRequest {
        repository_path: fixture.repository.clone(),
        worktree_path: fixture.worktree(&format!("worktree-{suffix}")),
        worktree_id: format!("node-{suffix}"),
        reservation_id: format!("reservation:{suffix}"),
        attempt_id: format!("attempt:{suffix}"),
        path_identity: format!("workspace:fixture-{suffix}"),
        base_commit_sha: base.to_owned(),
        branch: format!("node-{suffix}"),
        allowed_write_paths,
    }
}

fn reservation(suffix: &str) -> ResourceReservation {
    let declaration = ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.to_owned(),
        schema_version: 0,
        declaration_id: format!("declaration:{suffix}"),
        runtime_graph_id: "runtime_graph:fixture".to_owned(),
        runtime_graph_content_hash: "a".repeat(64),
        node_id: format!("node:{suffix}"),
        claims: vec![casegraphen::execution_topology::ResourceClaim {
            resource: format!("git-worktree:node-{suffix}"),
            mode: ResourceMode::Exclusive,
            rate_limit_group: None,
            workspace_strategy: Some(WorkspaceStrategy::IsolatedWorktree),
            network_scope: vec![],
            secret_scope: vec![],
        }],
    };
    ResourceReservation {
        schema: RESOURCE_RESERVATION_SCHEMA.to_owned(),
        schema_version: 0,
        reservation_id: format!("reservation:{suffix}"),
        declaration_id: declaration.declaration_id.clone(),
        attempt_id: format!("attempt:{suffix}"),
        granted_at: "2026-08-03T00:00:00Z".to_owned(),
        grants: declaration_grants(&declaration),
    }
}

fn release_assertion(suffix: &str) -> ReservationDispositionAssertion {
    ReservationDispositionAssertion {
        schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
        schema_version: 0,
        assertion_id: format!("assertion:release-{suffix}"),
        reservation_id: format!("reservation:{suffix}"),
        attempt_id: format!("attempt:{suffix}"),
        kind: ReservationAssertionKind::Release,
        asserted_by: "operator:fixture".to_owned(),
        reason: "fixture holder completion independently observed".to_owned(),
        superseding_reservation_id: None,
    }
}

fn git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .expect("run git");
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

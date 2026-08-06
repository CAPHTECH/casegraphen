#![allow(missing_docs)]

//! `casegraphen github observe|refresh|project` driven through the real
//! binary (project rule: a new command needs a test through the binary, not
//! only a unit test), against the frozen dogfood pilot corpus
//! `docs/pilots/issue-102/source/` (PR 101, `CAPHTECH/casegraphen`).
//!
//! `tests/cli_surface.rs` already proves every documented `github` path
//! fails with a missing-argument usage error, and `tests/product_surface.rs`
//! proves the three commands never create or modify a file. This file proves
//! the behavior: exact ground truth on replay, `head_unchanged` on an
//! unmoved capture, a tampered `--previous-observation` basis refusing
//! before `classify_refresh` ever runs, a clean projection with the two
//! declared residual risks, `--require-independent-review` turning into a
//! blocking finding, and byte-identical output on repeated runs.

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn pilot_dir() -> PathBuf {
    root().join("docs/pilots/issue-102")
}

fn manifest_path() -> PathBuf {
    pilot_dir().join("capture_manifest.v0.json")
}

/// `tests/fixtures/github-evidence/<case>/` — the adversarial capture
/// fixtures (design §10.2 / T6). Every fixture under here is a *separate*
/// mutated copy; the frozen pilot corpus `docs/pilots/issue-102/source/` is
/// never written to build one.
fn fixture_dir(case: &str) -> PathBuf {
    root().join("tests/fixtures/github-evidence").join(case)
}

fn observe_output(manifest: &Path, capture_dir: &Path) -> Output {
    run(&[
        "github",
        "observe",
        "--manifest",
        manifest.to_str().unwrap(),
        "--capture-dir",
        capture_dir.to_str().unwrap(),
        "--format",
        "json",
    ])
}

fn observe(manifest: &Path, capture_dir: &Path) -> Value {
    let output = observe_output(manifest, capture_dir);
    assert!(
        output.status.success(),
        "casegraphen github observe stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("casegraphen github observe stdout JSON")
}

fn project(manifest: &Path, capture_dir: &Path, require_independent_review: bool) -> Value {
    let mut args = vec![
        "github",
        "project",
        "--manifest",
        manifest.to_str().unwrap(),
        "--capture-dir",
        capture_dir.to_str().unwrap(),
    ];
    if require_independent_review {
        args.push("--require-independent-review");
    }
    args.extend(["--format", "json"]);
    let output = run(&args);
    assert!(
        output.status.success(),
        "casegraphen github project stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("casegraphen github project stdout JSON")
}

fn refresh(
    manifest: &Path,
    capture_dir: &Path,
    previous_manifest: &Path,
    previous_capture_dir: &Path,
) -> Value {
    let output = run(&[
        "github",
        "refresh",
        "--manifest",
        manifest.to_str().unwrap(),
        "--capture-dir",
        capture_dir.to_str().unwrap(),
        "--previous-manifest",
        previous_manifest.to_str().unwrap(),
        "--previous-capture-dir",
        previous_capture_dir.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "casegraphen github refresh stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("casegraphen github refresh stdout JSON")
}

fn evidence_role_of<'a>(independence: &'a Value, subject_id: &str) -> &'a str {
    independence["classifications"]
        .as_array()
        .unwrap()
        .iter()
        .find(|classification| classification["subject_id"] == subject_id)
        .unwrap_or_else(|| panic!("no classification for {subject_id}"))["evidence_role"]
        .as_str()
        .unwrap()
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "casegraphen-github-evidence-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create fixture directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run `casegraphen {}`: {error}", args.join(" ")))
}

fn run_ok(args: &[&str]) -> Value {
    let output = run(args);
    assert!(
        output.status.success(),
        "casegraphen {} stderr: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("casegraphen {} stdout JSON: {error}", args.join(" ")))
}

fn observe_pilot() -> Value {
    run_ok(&[
        "github",
        "observe",
        "--manifest",
        manifest_path().to_str().unwrap(),
        "--capture-dir",
        pilot_dir().to_str().unwrap(),
        "--format",
        "json",
    ])
}

fn project_pilot_output(require_independent_review: bool) -> Output {
    let manifest = manifest_path();
    let capture_dir = pilot_dir();
    let mut args = vec![
        "github",
        "project",
        "--manifest",
        manifest.to_str().unwrap(),
        "--capture-dir",
        capture_dir.to_str().unwrap(),
    ];
    if require_independent_review {
        args.push("--require-independent-review");
    }
    args.extend(["--format", "json"]);
    run(&args)
}

fn project_pilot(require_independent_review: bool) -> Value {
    let output = project_pilot_output(require_independent_review);
    assert!(
        output.status.success(),
        "casegraphen github project stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("casegraphen github project stdout JSON")
}

#[test]
fn observe_reproduces_the_documented_pilot_ground_truth() {
    let report = observe_pilot();
    let result = &report["result"];

    assert_eq!(report["metadata"]["command"], "casegraphen github observe");
    assert_eq!(result["accepted"], false);
    assert_eq!(result["mutation_performed"], false);
    assert_eq!(result["domain_findings"], serde_json::json!([]));

    let observation = &result["pr_observation"];
    assert_eq!(observation["repository"], "CAPHTECH/casegraphen");
    assert_eq!(observation["pr"]["number"], 101);
    assert_eq!(
        observation["base"]["sha"],
        "947f347f219a60775bcf71b226ce778cc8ea21f4"
    );
    assert_eq!(
        observation["head"]["sha"],
        "c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b"
    );
    assert_eq!(observation["liveness"]["state"], "MERGED");
    // Three-state provider enum, never coerced to a boolean: GitHub stops
    // reporting mergeability after merge.
    assert_eq!(observation["liveness"]["mergeable"], "UNKNOWN");
    assert_eq!(observation["changed_files"].as_array().unwrap().len(), 78);
    assert_eq!(
        observation["implementation_actors"]["actor_ids"],
        serde_json::json!(["MDQ6VXNlcjc5MDUxMQ=="])
    );

    let checks = result["check_evidence"].as_array().unwrap();
    assert_eq!(checks.len(), 3);
    let quality_successes = checks
        .iter()
        .filter(|check| check["kind"] == "check_run" && check["name"] == "quality")
        .count();
    assert_eq!(quality_successes, 2);
    assert!(checks
        .iter()
        .all(|check| check["kind"] != "check_run" || check["conclusion"] == "SUCCESS"));
    let status_context = checks
        .iter()
        .find(|check| check["kind"] == "status_context")
        .expect("one status_context check");
    assert_eq!(status_context["name"], "CodeRabbit");
    assert_eq!(status_context["state"], "SUCCESS");
    assert_eq!(status_context["description"], "Review rate limited");
    assert_eq!(status_context["creator"]["login"], "coderabbitai");
    assert_eq!(status_context["creator"]["typename"], "Bot");

    let findings = result["review_findings"].as_array().unwrap();
    let thread_ids: std::collections::BTreeSet<&str> = findings
        .iter()
        .filter_map(|finding| finding["thread"]["thread_id"].as_str())
        .collect();
    assert_eq!(thread_ids.len(), 9, "nine distinct review threads");
    assert!(
        findings
            .iter()
            .filter_map(|finding| finding["thread"].as_object())
            .all(|thread| thread["resolved"] == true),
        "every thread is resolved"
    );
    let edited_count = findings
        .iter()
        .filter(|finding| finding["edited"] == true)
        .count();
    assert_eq!(edited_count, 9, "nine edited findings");

    let resolvers: std::collections::BTreeMap<&str, &str> = findings
        .iter()
        .filter_map(|finding| finding["thread"]["resolved_by"].as_object())
        .map(|resolved_by| {
            (
                resolved_by["login"].as_str().unwrap(),
                resolved_by["id"].as_str().unwrap(),
            )
        })
        .collect();
    // The corpus's own two-logins-one-id fact: the thread resolver identity
    // `coderabbitai[bot]` carries the same node id as review/comment author
    // `coderabbitai` — one actor under two logins, id-keyed.
    assert_eq!(resolvers.get("rizumita"), Some(&"MDQ6VXNlcjc5MDUxMQ=="));
    assert_eq!(resolvers.get("coderabbitai[bot]"), Some(&"BOT_kgDOCCSy2w"));

    for finding in findings {
        let login = finding["author"]["login"].as_str().unwrap_or_default();
        let role = result["independence"]["classifications"]
            .as_array()
            .unwrap()
            .iter()
            .find(|classification| classification["subject_id"] == finding["finding_id"])
            .map(|classification| classification["evidence_role"].as_str().unwrap());
        if login == "rizumita" {
            assert_eq!(role, Some("self_review"), "rizumita subject {login}");
        } else if login == "coderabbitai" {
            assert_eq!(role, Some("automated_bot"), "coderabbitai subject {login}");
        }
    }

    // Review-summary findings collapse by `(author.id, body_content_hash,
    // path)` (design §3.4/acceptance criterion 7), so the 20 real reviews
    // land in 4 findings whose `duplicate_count`s preserve the original
    // count — the per-review `commit.oid` normalized verbatim survives that
    // collapse only if counted through `duplicate_count`, not by finding
    // count.
    let commit_sha_review_counts: std::collections::BTreeMap<String, u64> = findings
        .iter()
        .filter(|finding| finding["kind"] == "review_summary")
        .filter_map(|finding| {
            finding["commit_sha"]
                .as_str()
                .map(|sha| (sha.to_owned(), finding["duplicate_count"].as_u64().unwrap()))
        })
        .fold(std::collections::BTreeMap::new(), |mut counts, (sha, n)| {
            *counts.entry(sha).or_insert(0) += n;
            counts
        });
    assert_eq!(
        commit_sha_review_counts.get("c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b"),
        Some(&19)
    );
    assert_eq!(
        commit_sha_review_counts.get("5403673f13b45d8deb0f4be62f50390172071bb0"),
        Some(&1)
    );
    assert_eq!(
        commit_sha_review_counts.values().sum::<u64>(),
        20,
        "all 20 reviews accounted for"
    );

    assert_eq!(
        result["independence"]["independent_human_approvals"],
        serde_json::json!([])
    );
    assert_eq!(result["independence"]["independence_proven"], false);
}

fn refresh_pilot_against_previous(previous_manifest: &Path, previous_capture_dir: &Path) -> Output {
    run(&[
        "github",
        "refresh",
        "--manifest",
        manifest_path().to_str().unwrap(),
        "--capture-dir",
        pilot_dir().to_str().unwrap(),
        "--previous-manifest",
        previous_manifest.to_str().unwrap(),
        "--previous-capture-dir",
        previous_capture_dir.to_str().unwrap(),
        "--format",
        "json",
    ])
}

#[test]
fn refresh_against_the_same_capture_reports_head_unchanged() {
    let output = refresh_pilot_against_previous(&manifest_path(), &pilot_dir());
    assert!(
        output.status.success(),
        "a head-unchanged refresh must not change the exit code: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("refresh stdout JSON");
    let result = &report["result"];
    assert_eq!(result["accepted"], false);
    assert_eq!(result["mutation_performed"], false);
    assert_eq!(result["refresh_result"]["disposition"], "head_unchanged");
    assert_eq!(result["refresh_result"]["review_basis_moved"], false);
    assert_eq!(
        result["refresh_result"]["observation_changes"],
        serde_json::json!([])
    );
    assert!(result["refresh_result"]["refreshed_observation_hash"].is_string());
    // Negative path of the domain-finding channel: an unmoved review basis
    // reports nothing through it.
    assert_eq!(result["domain_findings"], serde_json::json!([]));
}

/// A minimal, self-consistent synthetic capture (same shape T2's own
/// `normalize.rs` unit tests build — a `pr`/`files`/`reviews`/
/// `review_threads`/`commits`/`checks` sextet with no findings) at a head
/// distinct from the pilot's real `c9be9ed6…`. Used only to drive a genuine
/// `stale_head` refresh through the real binary — the pilot corpus alone
/// only ever offers one head, so this is the minimum synthetic capture that
/// still passes `normalize`'s own intra-capture consistency checks (the
/// `checks` artifact's `headRefOid` must agree with the `pr` artifact's, and
/// likewise for `review_threads`'s `baseRefOid`/`headRefOid`).
fn write_stale_head_previous_capture(dir: &Path) -> PathBuf {
    const REPO: &str = "CAPHTECH/casegraphen";
    const BASE_SHA: &str = "947f347f219a60775bcf71b226ce778cc8ea21f4";
    const HEAD_SHA: &str = "cccccccccccccccccccccccccccccccccccccccc";

    let write = |name: &str, value: &Value| -> (String, String) {
        let bytes = serde_json::to_vec(value).unwrap();
        fs::write(dir.join(name), &bytes).unwrap();
        (name.to_owned(), format!("sha256:{}", sha256_hex(&bytes)))
    };

    let pr = serde_json::json!({
        "number": 101, "title": "x", "url": format!("https://github.com/{REPO}/pull/101"),
        "state": "OPEN", "author": {"id": "actor:pr-author", "login": "alice"},
        "baseRefName": "main", "baseRefOid": BASE_SHA,
        "headRefName": "feature", "headRefOid": HEAD_SHA,
        "createdAt": "2026-01-01T00:00:00Z", "mergedAt": null,
        "mergeable": "MERGEABLE", "mergeStateStatus": "CLEAN", "body": "pr body"
    });
    let files = serde_json::json!({"files": [{"path": "a.rs", "additions": 1, "deletions": 0, "changeType": "ADDED"}]});
    let reviews = serde_json::json!({"data": {"repository": {"pullRequest": {
        "number": 101, "reviews": {"totalCount": 0, "nodes": []}
    }}}});
    let review_threads = serde_json::json!({"data": {"repository": {"pullRequest": {
        "number": 101, "baseRefOid": BASE_SHA, "headRefOid": HEAD_SHA,
        "reviewThreads": {"totalCount": 0, "nodes": []}
    }}}});
    let commits = serde_json::json!({"data": {"repository": {"pullRequest": {"commits": {"nodes": [
        {"commit": {
            "author": {"user": {"__typename": "User", "login": "alice", "id": "actor:pr-author"}},
            "committer": {"user": {"__typename": "User", "login": "alice", "id": "actor:pr-author"}}
        }}
    ]}}}}});
    let checks = serde_json::json!({"data": {"repository": {"pullRequest": {
        "headRefOid": HEAD_SHA,
        "commits": {"nodes": [{"commit": {"oid": HEAD_SHA, "statusCheckRollup": {"contexts": {"nodes": []}}}}]}
    }}}});

    let (pr_path, pr_hash) = write("pr.json", &pr);
    let (files_path, files_hash) = write("files.json", &files);
    let (reviews_path, reviews_hash) = write("reviews.json", &reviews);
    let (threads_path, threads_hash) = write("review_threads.json", &review_threads);
    let (commits_path, commits_hash) = write("commits.json", &commits);
    let (checks_path, checks_hash) = write("checks.json", &checks);

    let entry = |category: &str, path: &str, hash: &str| {
        serde_json::json!({
            "category": category, "artifact_path": path, "content_hash": hash, "command_record": []
        })
    };
    let manifest = serde_json::json!({
        "schema": "casegraphen.experimental.github.capture_manifest.v0",
        "repository": REPO, "issue_numbers": [], "pr_number": 101,
        "captured_at": "2026-01-01T00:00:00Z", "capture_tool": "gh",
        "entries": [
            entry("pr", &pr_path, &pr_hash),
            entry("files", &files_path, &files_hash),
            entry("reviews", &reviews_path, &reviews_hash),
            entry("review_threads", &threads_path, &threads_hash),
            entry("commits", &commits_path, &commits_hash),
            entry("checks", &checks_path, &checks_hash),
        ]
    });
    let manifest_path = dir.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    manifest_path
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// The one condition `github refresh` exists to detect — a review basis
/// that no longer names the same head, base, PR, or repository — must be
/// visible through `result.domain_findings`, not only inside
/// `result.refresh_result.disposition`. An earlier version of this command
/// computed the process-level domain-finding flag from `disposition` but
/// never surfaced it in `domain_findings` (which the field did not even
/// exist for `refresh`), so a scripted caller checking that field the same
/// way it would check `memory_check`'s `findings` field saw nothing.
#[test]
fn refresh_against_a_moved_head_reports_a_domain_finding() {
    let directory = TestDirectory::create("stale-head");
    let previous_manifest = write_stale_head_previous_capture(directory.path());

    let output = refresh_pilot_against_previous(&previous_manifest, directory.path());
    assert!(
        output.status.success(),
        "a domain finding must not change the exit code (no --strict on github commands): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("refresh stdout JSON");
    let result = &report["result"];

    assert_eq!(result["refresh_result"]["disposition"], "stale_head");
    assert_eq!(result["refresh_result"]["review_basis_moved"], false);
    assert!(result["refresh_result"]["refreshed_observation_hash"].is_null());

    let domain_findings = result["domain_findings"].as_array().unwrap();
    assert!(
        !domain_findings.is_empty(),
        "a stale-head refresh must be visible in result.domain_findings, not only inside \
         result.refresh_result.disposition"
    );
    assert!(domain_findings
        .iter()
        .any(|finding| finding["code"] == "stale_head"));
}

#[test]
fn refresh_accepts_a_matching_declared_previous_observation() {
    let directory = TestDirectory::create("declared-basis");
    let observed = observe_pilot();
    let previous_observation_path = directory.path().join("previous_observation.json");
    fs::write(
        &previous_observation_path,
        serde_json::to_vec(&observed["result"]["pr_observation"]).unwrap(),
    )
    .expect("write declared previous observation");

    let report = run_ok(&[
        "github",
        "refresh",
        "--manifest",
        manifest_path().to_str().unwrap(),
        "--capture-dir",
        pilot_dir().to_str().unwrap(),
        "--previous-manifest",
        manifest_path().to_str().unwrap(),
        "--previous-capture-dir",
        pilot_dir().to_str().unwrap(),
        "--previous-observation",
        previous_observation_path.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert_eq!(
        report["result"]["refresh_result"]["disposition"],
        "head_unchanged"
    );
}

/// The T5 resolution (design doc §9, §6.1) is stronger than a bare
/// content-hash re-check: it refuses a `--previous-observation` whose
/// declared fields do not match the observation re-normalized from
/// `--previous-manifest`/`--previous-capture-dir`, which catches even a
/// **fully self-consistent forgery** (a tampered head SHA with its
/// `normalized_content_hash` recomputed to match) — design §6.1 is explicit
/// that a bare hash-recompute check alone proves only self-consistency, not
/// provenance.
#[test]
fn refresh_hard_refuses_a_declared_basis_that_disagrees_with_the_retained_capture() {
    let directory = TestDirectory::create("tampered-basis");
    let observed = observe_pilot();
    let mut tampered = observed["result"]["pr_observation"].clone();
    tampered["head"]["sha"] = serde_json::json!("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
    // Recompute nothing: even if an attacker forged the hash field to match
    // the tampered head (a self-consistent forgery), the CLI's equality
    // check against the retained previous-capture bytes still refuses,
    // because it never trusts the declared hash field as proof.
    let tampered_path = directory.path().join("tampered_observation.json");
    fs::write(&tampered_path, serde_json::to_vec(&tampered).unwrap())
        .expect("write tampered previous observation");

    let output = run(&[
        "github",
        "refresh",
        "--manifest",
        manifest_path().to_str().unwrap(),
        "--capture-dir",
        pilot_dir().to_str().unwrap(),
        "--previous-manifest",
        manifest_path().to_str().unwrap(),
        "--previous-capture-dir",
        pilot_dir().to_str().unwrap(),
        "--previous-observation",
        tampered_path.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not match") && stderr.contains("integrity failure"),
        "refusal did not name the basis-integrity failure: {stderr}"
    );
}

#[test]
fn project_pilot_has_no_blocking_findings_but_declares_the_two_residual_risks() {
    let report = project_pilot(false);
    let result = &report["result"];
    assert_eq!(result["accepted"], false);
    assert_eq!(result["mutation_performed"], false);
    assert_eq!(
        result["projection"]["blocking_findings"],
        serde_json::json!([])
    );
    let residual_codes: std::collections::BTreeSet<&str> = result["projection"]["residual_risks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|risk| risk["code"].as_str().unwrap())
        .collect();
    assert!(residual_codes.contains("no_independent_human_approval"));
    assert!(residual_codes.contains("status_context_description"));
    assert_eq!(result["projection"]["failed_checks"], serde_json::json!([]));
    assert_eq!(result["domain_findings"], serde_json::json!([]));
}

/// Pins the flagged and unflagged paths apart on the one channel a scripted
/// caller actually reads: `result.domain_findings`. `result.projection
/// .blocking_findings` alone is not enough — an earlier version of this
/// command set the process-level domain-finding flag from
/// `blocking_findings` without ever surfacing it in `domain_findings`, so an
/// automated caller checking only that field (the same field
/// `memory_check`'s `findings` plays the equivalent role for) saw an empty
/// list and no obstruction even though the policy was unmet. Neither path
/// changes the process exit code — `github` commands have no `--strict`
/// flag, exactly like `memory check`, so a domain finding is reported
/// through the JSON body only, never through a non-zero exit — which this
/// test also pins down so a future change cannot introduce a silent
/// exit-code divergence between the two paths.
#[test]
fn project_with_require_independent_review_reports_a_domain_finding() {
    let flagged = project_pilot_output(true);
    assert!(
        flagged.status.success(),
        "a domain finding must not change the exit code (no --strict on github commands): {}",
        String::from_utf8_lossy(&flagged.stderr)
    );
    let flagged_report: Value =
        serde_json::from_slice(&flagged.stdout).expect("flagged run stdout JSON");
    let flagged_result = &flagged_report["result"];

    let blocking = flagged_result["projection"]["blocking_findings"]
        .as_array()
        .unwrap();
    assert!(
        !blocking.is_empty(),
        "unmet --require-independent-review must be a blocking finding"
    );
    assert!(blocking.iter().any(|finding| finding["finding_id"]
        .as_str()
        .unwrap()
        .starts_with("independent_review_policy:")));

    let domain_findings = flagged_result["domain_findings"].as_array().unwrap();
    assert!(
        !domain_findings.is_empty(),
        "the unmet policy obstruction must also be visible in result.domain_findings, \
         not only inside result.projection.blocking_findings"
    );
    assert!(domain_findings.iter().any(|finding| finding["detail"]
        .as_str()
        .unwrap()
        .contains("require_independent_review")));

    // Unflagged: the same pilot capture with no independent-review demand
    // reports no domain finding through either channel.
    let unflagged = project_pilot_output(false);
    assert!(unflagged.status.success());
    let unflagged_report: Value =
        serde_json::from_slice(&unflagged.stdout).expect("unflagged run stdout JSON");
    assert_eq!(
        unflagged_report["result"]["domain_findings"],
        serde_json::json!([])
    );
}

#[test]
fn observe_and_project_are_byte_identical_on_repeated_runs() {
    let directory = TestDirectory::create("replay");
    for command in ["observe", "project"] {
        let first = directory.path().join(format!("{command}-1.json"));
        let second = directory.path().join(format!("{command}-2.json"));
        for output_path in [&first, &second] {
            let status = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
                .args([
                    "github",
                    command,
                    "--manifest",
                    manifest_path().to_str().unwrap(),
                    "--capture-dir",
                    pilot_dir().to_str().unwrap(),
                    "--format",
                    "json",
                    "--output",
                    output_path.to_str().unwrap(),
                ])
                .status()
                .expect("run casegraphen github command with --output");
            assert!(status.success(), "casegraphen github {command} --output");
        }
        let first_bytes = fs::read(&first).expect("read first replay output");
        let second_bytes = fs::read(&second).expect("read second replay output");
        assert_eq!(
            first_bytes, second_bytes,
            "casegraphen github {command} must be byte-identical on repeated runs"
        );
    }
}

fn expected_dir() -> PathBuf {
    pilot_dir().join("expected")
}

fn read_expected(name: &str) -> Value {
    let text = fs::read_to_string(expected_dir().join(name))
        .unwrap_or_else(|error| panic!("read docs/pilots/issue-102/expected/{name}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {name}: {error}"))
}

/// Design §5's replay property (E3), the stronger form: not just two runs
/// agreeing with *each other* inside one test process
/// (`observe_and_project_are_byte_identical_on_repeated_runs` above already
/// proves that), but a completely fresh normalization from the retained
/// source bytes reproducing the exact hashes retained in
/// `docs/pilots/issue-102/expected/` — the artifacts this repository commits
/// as the acceptance oracle, not a value this test process computed for
/// itself moments earlier. There is nothing to "delete" here beyond the
/// process itself: `github observe`/`github project` already recompute
/// everything from `--manifest`/`--capture-dir` on every invocation (no
/// intermediate bundle file, design §9), so a fresh process *is* the
/// delete-and-rebuild the acceptance criterion asks for.
#[test]
fn rebuild_from_retained_source_matches_retained_expected_hashes() {
    let observed = observe_pilot();
    let result = &observed["result"];

    let expected_observation = read_expected("pr_observation.json");
    assert_eq!(
        result["pr_observation"]["normalized_content_hash"],
        expected_observation["normalized_content_hash"],
        "a fresh normalize() of the retained source must reproduce the retained \
         pr_observation.normalized_content_hash exactly"
    );
    assert_eq!(result["pr_observation"], expected_observation);

    let expected_checks = read_expected("check_evidence.json");
    assert_eq!(result["check_evidence"], expected_checks);

    let expected_findings = read_expected("review_findings.json");
    assert_eq!(result["review_findings"], expected_findings);

    let expected_independence = read_expected("review_independence.json");
    assert_eq!(result["independence"], expected_independence);

    let projected = project_pilot(false);
    let expected_projection = read_expected("review_projection.json");
    assert_eq!(
        projected["result"]["projection"]["projection_content_hash"],
        expected_projection["projection_content_hash"],
        "a fresh project_review() of the retained source must reproduce the retained \
         review_projection.projection_content_hash exactly"
    );
    assert_eq!(projected["result"]["projection"], expected_projection);

    let refreshed = refresh(
        &manifest_path(),
        &pilot_dir(),
        &manifest_path(),
        &pilot_dir(),
    );
    let expected_refresh = read_expected("refresh_result.json");
    assert_eq!(refreshed["result"]["refresh_result"], expected_refresh);
}

// ---------------------------------------------------------------------
// Adversarial fixtures (design doc §10.2, IMPLEMENTATION-PLAN.md T6,
// acceptance criterion A11). Every fixture lives under
// `tests/fixtures/github-evidence/<case>/` as a *separate* capture, never as
// an edit to the frozen pilot corpus `docs/pilots/issue-102/source/`. Cases
// derived from the real pilot bytes (mutated copies, or the pilot's own
// unattested `reviews` surplus) are built by
// `tests/fixtures/github-evidence/generate_pilot_derived.py`; the minimal
// synthetic sextet captures (the same six-artifact shape this file's own
// `write_stale_head_previous_capture` already uses) are built by
// `tests/fixtures/github-evidence/generate_synthetic.py`. Both scripts'
// output is committed here as static fixture data — determinism (design §5)
// means regenerating them reproduces the same bytes; regeneration is not
// part of the test or build gate.
// ---------------------------------------------------------------------

/// Team-lead case 1, measured against the real binary: all 10 `coderabbitai`
/// reviews in the pilot corpus flipped to `state: APPROVED` and rebound to
/// the observed head, manifest rehashed. Bulk bot approval at the exact head
/// still cannot satisfy an independent-review requirement — the classifier
/// carries the guarantee, not the accident of which review states a bot
/// happened to submit (design §6, "Had CodeRabbit submitted APPROVED
/// instead of COMMENTED...").
#[test]
fn bot_approval_at_head_never_satisfies_independent_review() {
    let dir = fixture_dir("bot-approval-at-head");
    let manifest = dir.join("capture_manifest.v0.json");

    let observed = observe(&manifest, &dir);
    let independence = &observed["result"]["independence"];
    let mut roles: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for classification in independence["classifications"].as_array().unwrap() {
        *roles
            .entry(classification["evidence_role"].as_str().unwrap())
            .or_insert(0) += 1;
    }
    assert_eq!(roles.get("automated_bot"), Some(&20));
    assert_eq!(roles.get("self_review"), Some(&11));
    assert_eq!(roles.get("ci_check"), Some(&3));
    assert_eq!(
        independence["independent_human_approvals"],
        serde_json::json!([])
    );
    assert_eq!(independence["independence_proven"], false);

    let projected = project(&manifest, &dir, true);
    assert_eq!(
        projected["result"]["independence"]["policy"]["satisfied"],
        false
    );
}

/// Team-lead cases 2 and 3, paired so they differ in exactly one field: one
/// synthetic outside-human `APPROVED` review appended to the pilot's real
/// reviews artifact — `__typename: "User"`, a node id outside the
/// implementation actor set, `authorAssociation: NONE` — proves the only
/// satisfying shape on real corpus data. Without this fixture the suite
/// cannot tell a correct classifier from one that always answers "not
/// satisfied" (every other fixture in this file is a refusal/exclusion
/// path).
#[test]
fn positive_control_outside_human_approval_at_head_satisfies() {
    let dir = fixture_dir("positive-control-outside-approval");
    let manifest = dir.join("capture_manifest.v0.json");

    let observed = observe(&manifest, &dir);
    let independence = &observed["result"]["independence"];
    let approvals = independence["independent_human_approvals"]
        .as_array()
        .unwrap();
    assert_eq!(approvals.len(), 1, "exactly the synthetic outside approval");
    let finding_id = approvals[0].as_str().unwrap();
    assert_eq!(
        evidence_role_of(independence, finding_id),
        "independent_human_candidate"
    );
    assert_eq!(independence["excluded_approvals"], serde_json::json!([]));
    // independence_proven never flips, even when a demand is satisfied.
    assert_eq!(independence["independence_proven"], false);

    let projected = project(&manifest, &dir, true);
    assert_eq!(
        projected["result"]["independence"]["policy"]["satisfied"],
        true
    );
    assert_eq!(
        projected["result"]["independence"]["policy"]["satisfying_finding_ids"],
        serde_json::json!([finding_id])
    );
}

/// The pair to the positive control: the identical synthetic review,
/// rebound to the older commit `5403673f13b45d8deb0f4be62f50390172071bb0`
/// instead of the observed head — the only field that differs. Proves the
/// discrimination is on the commit binding and nothing else.
#[test]
fn positive_control_older_binding_is_excluded_not_credited() {
    let dir = fixture_dir("positive-control-older-binding");
    let manifest = dir.join("capture_manifest.v0.json");

    let observed = observe(&manifest, &dir);
    let independence = &observed["result"]["independence"];
    assert_eq!(
        independence["independent_human_approvals"],
        serde_json::json!([])
    );
    let excluded = independence["excluded_approvals"].as_array().unwrap();
    assert_eq!(excluded.len(), 1);
    assert_eq!(excluded[0]["reason"], "approval_not_bound_to_observed_head");

    let projected = project(&manifest, &dir, true);
    assert_eq!(
        projected["result"]["independence"]["policy"]["satisfied"],
        false
    );
}

/// Team-lead case 4: `trusted`, `approved`, `accepted`, `authority`,
/// `evidence_role`, `independence_proven`, and a bogus `review_state:
/// "APPROVED"` planted on all 20 real review nodes. The allowlist parser
/// reads only `state` (not the planted `review_state`), so classification is
/// entirely unreachable by the injected fields — identical role counts and
/// finding count to the clean pilot corpus.
#[test]
fn caller_declared_approval_fields_inside_provider_json_are_unread() {
    let dir = fixture_dir("caller-declared-approval-raw-fields");
    let manifest = dir.join("capture_manifest.v0.json");
    let mutated = observe(&manifest, &dir);
    let baseline = observe_pilot();

    let mutated_result = &mutated["result"];
    let baseline_result = &baseline["result"];
    assert_eq!(
        mutated_result["review_findings"].as_array().unwrap().len(),
        baseline_result["review_findings"].as_array().unwrap().len(),
    );
    assert_eq!(
        mutated_result["independence"]["classifications"],
        baseline_result["independence"]["classifications"],
        "planted trust/approval fields must not move a single classification"
    );
    assert_eq!(
        mutated_result["independence"]["independent_human_approvals"],
        serde_json::json!([])
    );
}

/// Team-lead case 5: `"trusted": true` at the top level of the caller-authored
/// manifest wrapper. `CaptureManifest` is `deny_unknown_fields`, so this is
/// refused before a single artifact byte is read.
#[test]
fn caller_declared_trust_in_manifest_wrapper_is_refused() {
    let manifest = fixture_dir("caller-declared-trust-manifest-wrapper").join("manifest.json");
    let output = observe_output(&manifest, &pilot_dir());
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown field") && stderr.contains("trusted"),
        "stderr: {stderr}"
    );
}

/// Design §10.2 row 4: a `reviews` artifact built from the pilot's own
/// unattested `reviews` surplus (`pr-101.json`'s own `reviews` array, whose
/// author objects are `{"login": ...}` only — real provider bytes; a `url`
/// per node is synthesized because the GraphQL reviews wire shape requires
/// one and this gh --json section does not carry it, which is the only
/// field added). All 20 authors classify `unattributed`, and — because the
/// collapse key requires actor-id equality — none of them collapse, so the
/// finding count grows relative to the clean (id-bearing) pilot corpus
/// rather than shrinking.
#[test]
fn missing_actor_attestation_classifies_unattributed_and_never_collapses() {
    let dir = fixture_dir("missing-actor-attestation");
    let manifest = dir.join("capture_manifest.v0.json");
    let observed = observe(&manifest, &dir);
    let result = &observed["result"];

    let review_summaries: Vec<&Value> = result["review_findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["kind"] == "review_summary")
        .collect();
    assert_eq!(
        review_summaries.len(),
        20,
        "no id on any review author means no duplicate collapse: all 20 reviews stay distinct"
    );
    assert!(review_summaries.iter().all(
        |finding| finding["author"]["id"].is_null() && finding["author"]["typename"].is_null()
    ));

    let independence = &result["independence"];
    for finding in &review_summaries {
        let role = evidence_role_of(independence, finding["finding_id"].as_str().unwrap());
        assert_eq!(role, "unattributed", "finding {}", finding["finding_id"]);
    }
    assert_eq!(
        independence["independent_human_approvals"],
        serde_json::json!([])
    );
}

/// Design §10.2 row 2 / T3's actor-substitution shape reproduced through the
/// real binary: an `APPROVED` review whose author node **id** equals a
/// commit author's id but whose **login** differs (a rename between
/// captures). The implementation actor set is id-keyed, so the rename does
/// not move the actor out of it — `self_review` fires via
/// `author_in_implementation_actor_set`, not a login match.
#[test]
fn actor_substitution_by_node_id_is_still_self_review() {
    let dir = fixture_dir("actor-substitution");
    let manifest = dir.join("manifest.json");
    let observed = observe(&manifest, &dir);
    let result = &observed["result"];

    let actor_ids: Vec<&str> = result["pr_observation"]["implementation_actors"]["actor_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    let finding = &result["review_findings"][0];
    let author_id = finding["author"]["id"].as_str().unwrap();
    assert!(
        actor_ids.contains(&author_id),
        "the review author's id must be in the implementation actor set (the rename is only on login)"
    );
    assert_ne!(
        finding["author"]["login"], "old-login",
        "the login on the review differs from the commit login (that's the rename)"
    );

    let independence = &result["independence"];
    assert_eq!(
        evidence_role_of(independence, finding["finding_id"].as_str().unwrap()),
        "self_review"
    );
    assert_eq!(
        independence["independent_human_approvals"],
        serde_json::json!([])
    );
}

/// Design §10.2 row 3: two otherwise-identical outside `APPROVED` reviews at
/// head, differing only in `authorAssociation` (`MEMBER` vs `NONE`). Identical
/// classification and policy result on both — `authorAssociation` is never an
/// input to any classifier arm.
#[test]
fn author_association_is_not_read_as_independence() {
    let dir = fixture_dir("association-not-independence");
    let manifest = dir.join("manifest.json");
    let observed = observe(&manifest, &dir);
    let result = &observed["result"];

    let findings = result["review_findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2);
    let independence = &result["independence"];
    for finding in findings {
        assert_eq!(
            evidence_role_of(independence, finding["finding_id"].as_str().unwrap()),
            "independent_human_candidate"
        );
    }
    let mut approvals: Vec<&str> = independence["independent_human_approvals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    approvals.sort_unstable();
    let mut finding_ids: Vec<&str> = findings
        .iter()
        .map(|finding| finding["finding_id"].as_str().unwrap())
        .collect();
    finding_ids.sort_unstable();
    assert_eq!(
        approvals, finding_ids,
        "both approvals satisfy identically regardless of authorAssociation"
    );
}

/// Design §10.2 row 5: the `commits` artifact's user objects lack GitHub
/// node ids, so the implementation actor set cannot be built. Hard refusal
/// (integrity class) — never a login fallback.
#[test]
fn actor_set_source_without_node_ids_is_a_hard_refusal() {
    let dir = fixture_dir("actor-set-source-without-ids");
    let manifest = dir.join("manifest.json");
    let output = observe_output(&manifest, &dir);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("actor_set_source_missing_id"),
        "stderr: {stderr}"
    );
}

/// Design §10.2 row 9: one `quality` check with conclusion `SKIPPED`. Never
/// folded into success — it lands in `inconclusive_checks`, Should Review,
/// and adds a `checks_inconclusive` residual risk, distinct from
/// `failed_checks`.
#[test]
fn skipped_check_is_inconclusive_never_success_never_failed() {
    let dir = fixture_dir("skipped-check");
    let manifest = dir.join("manifest.json");
    let projected = project(&manifest, &dir, false);
    let projection = &projected["result"]["projection"];

    assert_eq!(projection["failed_checks"], serde_json::json!([]));
    let inconclusive = projection["inconclusive_checks"].as_array().unwrap();
    assert_eq!(inconclusive.len(), 1);
    let check_id = inconclusive[0].as_str().unwrap();
    assert!(
        projection["should_review"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["subject_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == check_id)),
        "the skipped check must land in should_review"
    );
    let residual_codes: std::collections::BTreeSet<&str> = projection["residual_risks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|risk| risk["code"].as_str().unwrap())
        .collect();
    assert!(residual_codes.contains("checks_inconclusive"));
    assert_eq!(projection["blocking_findings"], serde_json::json!([]));
}

/// Design §10.2 row 11: two `Bot` thread comments identical under
/// `(author.id, body_content_hash, path)` in the same thread (one of them
/// the thread opener) collapse into one finding whose `duplicate_count`
/// preserves the original count; the actionable count is preserved (still
/// one actionable obligation, not two).
#[test]
fn duplicate_bot_findings_collapse_with_preserved_duplicate_count() {
    let dir = fixture_dir("duplicate-bot-findings");
    let manifest = dir.join("manifest.json");
    let observed = observe(&manifest, &dir);
    let findings = observed["result"]["review_findings"].as_array().unwrap();
    assert_eq!(
        findings.len(),
        1,
        "two identical comments collapse into one finding"
    );
    assert_eq!(findings[0]["duplicate_count"], 2);
    assert_eq!(findings[0]["actionable"], true);
}

/// Design §10.2 row 12: a thread comment URL naming a different repository
/// than the manifest declares. Excluded from `review_findings` and recorded
/// as a `cross_repository_reference` domain finding — never silently
/// included, never silently dropped.
#[test]
fn cross_repository_reference_is_excluded_and_declared_as_a_domain_finding() {
    let dir = fixture_dir("cross-repository-references");
    let manifest = dir.join("manifest.json");
    let observed = observe(&manifest, &dir);
    let result = &observed["result"];

    let findings = result["review_findings"].as_array().unwrap();
    assert_eq!(
        findings.len(),
        1,
        "the foreign-repository comment is excluded"
    );
    assert!(findings[0]["url"]
        .as_str()
        .unwrap()
        .starts_with("https://github.com/OWNER/repo/"));

    let domain_findings = result["domain_findings"].as_array().unwrap();
    assert!(domain_findings
        .iter()
        .any(|finding| finding["code"] == "cross_repository_reference"
            && finding["detail"].as_str().unwrap().contains("OTHER/other")));
}

/// The `review_projection.v0` record is a standalone artifact: it carries
/// its own `projection_content_hash`/`full_trace`/`read_only`/`accepted` and
/// is meant to be written out and handed to a reviewer on its own, without
/// the command envelope's `result.domain_findings` sitting next to it. A
/// consumer holding only that record must still be able to tell a finding
/// was excluded for pointing at another repository — that fact belongs in
/// `losses` (`omitted_refs` carries the excluded URL), not only in the
/// envelope-level channel. This test writes `--output` to a real file and
/// reads *only* `result.projection` back out of it, proving what a
/// downstream consumer of the written record — not the live command
/// envelope this test process just produced in memory — would see.
#[test]
fn the_projection_record_alone_declares_the_cross_repository_exclusion() {
    let directory = TestDirectory::create("cross-repo-projection-record");
    let dir = fixture_dir("cross-repository-references");
    let manifest = dir.join("manifest.json");
    let output_path = directory.path().join("projection.json");

    let status = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args([
            "github",
            "project",
            "--manifest",
            manifest.to_str().unwrap(),
            "--capture-dir",
            dir.to_str().unwrap(),
            "--format",
            "json",
            "--output",
            output_path.to_str().unwrap(),
        ])
        .status()
        .expect("run casegraphen github project --output");
    assert!(status.success());

    let written_text = fs::read_to_string(&output_path).expect("read written projection output");
    let written: Value = serde_json::from_str(&written_text).expect("written output is JSON");
    let record = written["result"]["projection"].clone();

    let losses = record["losses"].as_array().unwrap();
    let cross_repo_loss = losses
        .iter()
        .find(|loss| loss["loss_kind"] == "cross_repository_excluded")
        .unwrap_or_else(|| {
            panic!(
                "review_projection.v0's own losses must declare the cross-repository \
                 exclusion; losses: {losses:?}"
            )
        });
    assert!(cross_repo_loss["omitted_refs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|url| url.as_str().unwrap().contains("OTHER/other")));
    // Sanity: the record itself (not the envelope) carries this, so it is
    // reachable without ever looking at a sibling `domain_findings` field.
    assert!(record.get("domain_findings").is_none());
}

/// Design §10.2 row 8: same head, second capture missing a `quality`-sibling
/// check run (`lint`) present in the previous one.
#[test]
fn disappearing_check_is_reported_as_removed_on_refresh() {
    let base = fixture_dir("disappearing-checks");
    let previous_dir = base.join("previous");
    let current_dir = base.join("current");
    let refreshed = refresh(
        &current_dir.join("manifest.json"),
        &current_dir,
        &previous_dir.join("manifest.json"),
        &previous_dir,
    );
    let refresh_result = &refreshed["result"]["refresh_result"];
    assert_eq!(refresh_result["disposition"], "head_unchanged");
    let changes = refresh_result["observation_changes"].as_array().unwrap();
    assert!(
        changes.iter().any(|change| change["category"] == "checks"
            && change["change"] == "removed"
            && change["detail"].as_str().unwrap().contains("lint")),
        "observation_changes: {changes:?}"
    );
}

/// Design §10.2 row 10: same head, an existing finding's body changes
/// (`lastEditedAt` set) between two captures. `edited: true` in the current
/// capture's own finding, and the refresh reports `changed` with a differing
/// `body_content_hash`.
#[test]
fn edited_review_comment_is_reported_as_changed_on_refresh() {
    let base = fixture_dir("edited-review-comments");
    let previous_dir = base.join("previous");
    let current_dir = base.join("current");

    let current_observed = observe(&current_dir.join("manifest.json"), &current_dir);
    let current_finding = &current_observed["result"]["review_findings"][0];
    assert_eq!(current_finding["edited"], true);

    let refreshed = refresh(
        &current_dir.join("manifest.json"),
        &current_dir,
        &previous_dir.join("manifest.json"),
        &previous_dir,
    );
    let refresh_result = &refreshed["result"]["refresh_result"];
    assert_eq!(refresh_result["disposition"], "head_unchanged");
    let changes = refresh_result["observation_changes"].as_array().unwrap();
    assert!(
        changes
            .iter()
            .any(|change| change["category"] == "review_findings"
                && change["change"] == "changed"
                && change["subject_id"] == current_finding["finding_id"]),
        "observation_changes: {changes:?}"
    );
}

#[test]
fn an_unsupported_capture_manifest_schema_is_a_hard_refusal() {
    let directory = TestDirectory::create("bad-schema");
    let manifest_text = fs::read_to_string(manifest_path()).unwrap();
    let mut manifest: Value = serde_json::from_str(&manifest_text).unwrap();
    manifest["schema"] = serde_json::json!("casegraphen.experimental.github.capture_manifest.v1");
    let bad_manifest_path = directory.path().join("bad_schema_manifest.json");
    fs::write(&bad_manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let output = run(&[
        "github",
        "observe",
        "--manifest",
        bad_manifest_path.to_str().unwrap(),
        "--capture-dir",
        pilot_dir().to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unsupported capture manifest schema"),
        "stderr: {stderr}"
    );
}

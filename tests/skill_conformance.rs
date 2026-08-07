#![allow(missing_docs)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{json, Value};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "casegraphen-skill-conformance-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temporary fixture store");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn generated_skill_surface_is_current() {
    let output = Command::new("python3")
        .args(["scripts/skill-conformance.py", "--check"])
        .current_dir(root())
        .output()
        .expect("run Skill conformance checker");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn orchestration_handoff_fails_closed_for_open_seams_and_unresolved_evidence() {
    let schema = root().join("schemas/experimental/skill.orchestration_handoff.v0.schema.json");
    let example: Value = serde_json::from_str(
        &fs::read_to_string(
            root().join("schemas/experimental/skill.orchestration_handoff.v0.example.json"),
        )
        .unwrap(),
    )
    .unwrap();

    let mut cases = Vec::new();
    let mut open_seam_continues = example.clone();
    open_seam_continues["return_required"] = json!(false);
    open_seam_continues["next_action"] =
        json!({"kind": "invoke_task_skill", "task_skill": "casegraphen-operate"});
    cases.push(open_seam_continues);

    let mut unresolved_evidence_continues = example;
    unresolved_evidence_continues["seams"] = json!([]);
    unresolved_evidence_continues["unresolved_evidence"] =
        json!(["independent evidence review is missing"]);
    unresolved_evidence_continues["return_required"] = json!(false);
    unresolved_evidence_continues["next_action"] =
        json!({"kind": "invoke_task_skill", "task_skill": "casegraphen-audit"});
    cases.push(unresolved_evidence_continues);

    for (index, value) in cases.into_iter().enumerate() {
        let temporary = TemporaryDirectory::new();
        let instance = temporary
            .path()
            .join(format!("invalid-handoff-{index}.json"));
        fs::write(&instance, serde_json::to_vec(&value).unwrap()).unwrap();
        let output = Command::new("python3")
            .args(["-m", "jsonschema", "-i"])
            .arg(&instance)
            .arg(&schema)
            .current_dir(root())
            .output()
            .expect("run handoff schema validator");
        assert!(
            !output.status.success(),
            "invalid handoff {index} unexpectedly passed"
        );
    }
}

#[test]
fn removed_flag_fixture_fails_with_a_location_and_mismatch() {
    rejected_fixture(
        "tests/fixtures/skill-conformance/removed-flag.md",
        "removed-flag.md:4: flag '--removed-flag' is not accepted by `casegraphen space inspect`",
    );
}

/// Issue #111 acceptance criterion: a gate fails when skill text names a
/// contract identifier or schema filename the installation does not
/// provide. This fixture is the deliberate failure that proves the guard
/// added in `scripts/skill-conformance.py::available_schema_identity`
/// actually refuses, not merely that it happens to pass on real skill text.
#[test]
fn missing_schema_fixture_fails_with_a_location_and_identity() {
    let output = checker("tests/fixtures/skill-conformance/missing-schema.md");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "missing-schema.md:3: schema id 'highergraphen.case.does_not_exist.v1' is not \
             provided by `casegraphen schema get`"
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains(
            "missing-schema.md:4: schema file 'nonexistent.schema.json' is not provided by \
             `casegraphen schema get`"
        ),
        "{stderr}"
    );
}

#[test]
fn stale_status_and_halt_fixture_fail_with_locations() {
    let output = checker("tests/fixtures/skill-conformance/stale-vocabulary.md");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("stale-vocabulary.md:3: unknown operation status 'retired_status'"),
        "{stderr}"
    );
    assert!(
        stderr.contains("stale-vocabulary.md:5: unknown halt reason 'retired_halt'"),
        "{stderr}"
    );
}

#[test]
fn executable_skill_example_runs_against_a_temporary_store() {
    let temporary = TemporaryDirectory::new();
    let output = Command::new("sh")
        .arg("skills/casegraphen-operate/examples/fixture-read.sh")
        .arg(env!("CARGO_BIN_EXE_casegraphen"))
        .arg(root())
        .arg(temporary.path())
        .current_dir(root())
        .output()
        .expect("run executable Skill example");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for output_name in [
        "lift.json",
        "inspect.json",
        "frontier.json",
        "reason.txt",
        "validate.json",
        "obstructions.json",
    ] {
        assert!(
            temporary.path().join(output_name).is_file(),
            "missing {output_name}"
        );
    }
}

#[test]
fn every_skill_refuses_a_removed_responsibility_boundary() {
    let mutations = [
        (
            "casegraphen-design",
            "Produces proposal artifacts only",
            "Produces artifacts only",
        ),
        (
            "casegraphen-audit",
            "Never invoke a mutation, review, evidence, transition, worker, `run`, or",
            "Never invoke a worker",
        ),
        (
            "casegraphen-integrate",
            "Every proposal remains `unreviewed`; `accepted` remains false.",
            "Every proposal is processed.",
        ),
        (
            "casegraphen-operate",
            "Every durable mutation needs a valid operation gate.",
            "Durable mutations normally use a gate.",
        ),
        (
            "casegraphen-memory-query",
            "It is not a conversation store, and relevance is not authority.",
            "It is a conversation store.",
        ),
        (
            "casegraphen-memory-curate",
            "Produce proposals only.",
            "Produce memory.",
        ),
        (
            "casegraphen-memory-audit",
            "Never repair an audit finding by accepting a claim",
            "Repair an audit finding by accepting a claim",
        ),
        (
            "casegraphen-github-evidence",
            "A refresh never rebases a review basis.",
            "A refresh may rebase a review basis.",
        ),
        (
            "casegraphen-orchestrate",
            "These are explicit return seams.",
            "These seams may be crossed automatically.",
        ),
    ];

    for (skill, from, to) in mutations {
        let temporary = TemporaryDirectory::new();
        let source_path = root().join("skills").join(skill).join("SKILL.md");
        let source = fs::read_to_string(&source_path).expect("read shipped Skill");
        assert!(source.contains(from), "mutation source missing in {skill}");
        let fixture_path = temporary.path().join(format!("{skill}.md"));
        fs::write(&fixture_path, source.replacen(from, to, 1)).expect("write mutated Skill");

        let output = Command::new("python3")
            .arg("scripts/skill-conformance.py")
            .arg("--check-document")
            .arg(&fixture_path)
            .current_dir(root())
            .output()
            .expect("run Skill responsibility checker");
        assert!(
            !output.status.success(),
            "mutated {skill} unexpectedly passed"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("responsibility contract is missing"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn checker(fixture: &str) -> std::process::Output {
    Command::new("python3")
        .args(["scripts/skill-conformance.py", "--check-document", fixture])
        .current_dir(root())
        .output()
        .expect("run Skill fixture checker")
}

fn rejected_fixture(fixture: &str, expected: &str) {
    let output = checker(fixture);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(expected), "{stderr}");
}

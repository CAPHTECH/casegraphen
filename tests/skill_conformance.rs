#![allow(missing_docs)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

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
fn removed_flag_fixture_fails_with_a_location_and_mismatch() {
    rejected_fixture(
        "tests/fixtures/skill-conformance/removed-flag.md",
        "removed-flag.md:4: flag '--removed-flag' is not accepted by `casegraphen space inspect`",
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

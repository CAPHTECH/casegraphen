#![allow(missing_docs)]

use casegraphen::runtime_protocol::{
    parse_runtime_node_report, reconcile_runtime_reports, ExpectedRuntimeNode,
    RuntimeGraphExpectation, RuntimeNodeReport,
};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path, process::Command};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn example_report() -> RuntimeNodeReport {
    parse_runtime_node_report(include_str!(
        "../schemas/experimental/runtime.node_report.example.json"
    ))
    .expect("valid runtime report example")
}

#[test]
fn static_audit_invokes_the_shipped_graph_linter() {
    let input = root()
        .join("tests/fixtures/casegraphen-design/verifier-correlation/execution.topology.json");
    let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(["graph", "lint", "--input"])
        .arg(input)
        .args(["--format", "json"])
        .output()
        .expect("run shipped graph linter");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("graph lint JSON");
    assert_eq!(
        report["schema"],
        "casegraphen.experimental.graph_lint.report.v0"
    );
    assert!(report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .any(|finding| finding["classification"] == "heuristic"));

    let skill = fs::read_to_string(root().join("skills/casegraphen-audit/SKILL.md"))
        .expect("read audit Skill");
    assert_eq!(
        shell_casegraphen_commands(&skill),
        vec!["casegraphen graph lint --input execution.topology.json --format json \\"]
    );
}

#[test]
fn canonical_reconciliation_refuses_199_of_200_as_complete() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/runtime/199-of-200.json"))
        .expect("199-of-200 fixture");
    let expected_count = fixture["expected_node_count"].as_u64().unwrap() as usize;
    let reported_count = fixture["reported_node_count"].as_u64().unwrap() as usize;
    let base = example_report();
    let expectation = RuntimeGraphExpectation {
        runtime_graph_id: base.runtime_graph_id.clone(),
        runtime_graph_content_hash: base.runtime_graph_content_hash.clone(),
        nodes: (0..expected_count)
            .map(|index| ExpectedRuntimeNode {
                node_id: format!("node:{index:04}"),
                expected_output_schema_id: base.expected_output_schema_id.clone(),
            })
            .collect(),
    };
    let reports = (0..reported_count)
        .map(|index| report_for_node(&base, index))
        .collect::<Vec<_>>();

    let completeness = reconcile_runtime_reports(&expectation, &reports, &[]);
    assert_eq!(completeness.expected_node_count, 200);
    assert_eq!(completeness.actual_report_count, 199);
    assert_eq!(completeness.missing_report_count, 1);
    assert!(!completeness.complete);
    assert!(completeness
        .findings
        .iter()
        .any(|finding| finding.code == "missing_report"
            && finding.node_id.as_deref() == Some("node:0199")));
}

#[test]
fn audit_evidence_classes_are_distinct_and_runtime_identity_stays_untrusted() {
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/casegraphen-audit/evidence-classes.json"
    ))
    .expect("evidence-class fixture");
    let classes = fixture["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(|case| case["class"].as_str().expect("class"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        classes,
        BTreeSet::from([
            "deterministic_violation",
            "observation",
            "runtime_declared",
            "inference"
        ])
    );

    let boundary = fs::read_to_string(
        root().join("skills/casegraphen-audit/references/reporting-boundary.md"),
    )
    .expect("read reporting boundary");
    for class in &classes {
        assert!(boundary.contains(&format!("`{class}`")));
    }
    for field in ["identity", "model", "context"] {
        assert!(boundary.contains(field));
    }
    assert!(boundary.contains("runtime-declared, untrusted"));
}

#[test]
fn run_audit_names_the_single_completeness_derivation_and_no_manual_algorithm() {
    let skill = fs::read_to_string(root().join("skills/casegraphen-audit/SKILL.md"))
        .expect("read audit Skill");
    let run_reference =
        fs::read_to_string(root().join("skills/casegraphen-audit/references/run-audit.md"))
            .expect("read run audit reference");
    let skill_words = skill.split_whitespace().collect::<Vec<_>>().join(" ");
    let reference_words = run_reference
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(skill_words.contains("reconcile_runtime_reports"));
    assert!(reference_words.contains("reconcile_runtime_reports"));
    assert!(skill_words.contains("Do not count reports or reconstruct retry lineage"));
    assert!(reference_words.contains("Do not derive completeness locally"));
}

fn report_for_node(base: &RuntimeNodeReport, index: usize) -> RuntimeNodeReport {
    let mut report = base.clone();
    report.report_id = format!("runtime_report:{index:04}");
    report.node_id = format!("node:{index:04}");
    report.attempt_id = format!("attempt:{index:04}:1");
    report.input_artifact_ids = vec![format!("artifact:input:{index:04}")];
    report.output_artifact_ids = vec![format!("artifact:output:{index:04}")];
    report
}

fn shell_casegraphen_commands(markdown: &str) -> Vec<String> {
    let mut in_shell = false;
    let mut commands = Vec::new();
    for line in markdown.lines() {
        let stripped = line.trim();
        if stripped.starts_with("```") {
            in_shell = stripped == "```sh";
            continue;
        }
        if in_shell && stripped.starts_with("casegraphen ") {
            commands.push(stripped.to_owned());
        }
    }
    commands
}

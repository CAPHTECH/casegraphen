#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{
        parse_execution_topology, DeliveryMode, EdgeKind, ExecutionTopology, Provenance,
        ResourceMode, SideEffects, TopologyEdge, WorkspaceStrategy,
    },
    graph_lint::{
        lint_execution_topology, lint_execution_topology_with_verification_policies,
        FindingClassification, LintSeverity,
    },
    verification_policy::parse_verification_policy,
};
use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(relative: &str) -> ExecutionTopology {
    let path = root().join(relative);
    parse_execution_topology(&fs::read_to_string(&path).expect("read topology"))
        .unwrap_or_else(|findings| panic!("{}: {findings:?}", path.display()))
}

fn run_lint(input: &Path, format: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(["graph", "lint", "--input"])
        .arg(input)
        .args(["--format", format])
        .output()
        .expect("run graph lint")
}

#[test]
fn cli_json_is_deterministic_schema_covered_and_text_is_available() {
    let input = root().join("schemas/experimental/execution.topology.file-review.example.json");
    let left = run_lint(&input, "json");
    let right = run_lint(&input, "json");
    assert!(
        left.status.success(),
        "{}",
        String::from_utf8_lossy(&left.stderr)
    );
    assert_eq!(left.stdout, right.stdout);
    let report: Value = serde_json::from_slice(&left.stdout).expect("lint JSON");
    assert_eq!(
        report["schema"],
        "casegraphen.experimental.graph_lint.report.v0"
    );
    assert_eq!(report["report_version"], 0);
    assert_eq!(report["metrics"]["theoretical_parallel_width"], 2);
    assert_eq!(
        report["topology_content_hash"]
            .as_str()
            .expect("hash")
            .len(),
        64
    );
    assert!(report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .all(|finding| {
            finding["suggested_next_operation"]["operation"].is_string()
                && finding["suggested_next_operation"]["parameters"].is_object()
        }));
    assert!(!report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .any(|finding| finding["code"] == "redundant_reachability"));

    let text = run_lint(&input, "text");
    assert!(text.status.success());
    assert!(String::from_utf8_lossy(&text.stdout).contains("parallel width: 2"));

    let schema: Value = serde_json::from_str(
        &fs::read_to_string(root().join("schemas/experimental/graph_lint.report.v0.schema.json"))
            .expect("read report schema"),
    )
    .expect("report schema JSON");
    assert_eq!(schema["$id"], report["schema"]);
}

#[test]
fn twenty_independent_files_report_width_twenty() {
    let topology =
        fixture("tests/fixtures/casegraphen-design/independent-fanout/execution.topology.json");
    let report = lint_execution_topology(&topology);
    assert_eq!(report.metrics.theoretical_parallel_width, 20);
}

#[test]
fn actual_verification_policy_replaces_uninspectable_guess() {
    let mut topology = fixture("schemas/experimental/execution.topology.file-review.example.json");
    let policy = parse_verification_policy(include_str!(
        "../schemas/experimental/verification.policy.example.json"
    ))
    .unwrap();
    topology.verification_policy_ids = vec![policy.verification_policy_id.clone()];
    for node in &mut topology.nodes {
        if node.verification_policy_id.is_some() {
            node.verification_policy_id = Some(policy.verification_policy_id.clone());
        }
    }
    let policies =
        std::collections::BTreeMap::from([(policy.verification_policy_id.clone(), policy)]);
    let report = lint_execution_topology_with_verification_policies(&topology, &policies);
    assert!(!report
        .findings
        .iter()
        .any(|finding| finding.code == "verification_independence_uninspectable"));
    assert!(!report
        .findings
        .iter()
        .any(|finding| finding.code == "verification_policy_missing"));
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.code == "runtime_attestation_not_independence_proof"));

    let missing = lint_execution_topology_with_verification_policies(
        &topology,
        &std::collections::BTreeMap::new(),
    );
    assert!(missing.findings.iter().any(|finding| {
        finding.code == "verification_policy_missing"
            && finding.classification == FindingClassification::Deterministic
    }));
}

#[test]
fn unordered_exclusive_writers_are_a_deterministic_error() {
    let mut topology =
        fixture("tests/fixtures/casegraphen-design/same-file-collision/execution.topology.json");
    topology.edges.clear();
    let isolated = lint_execution_topology(&topology);
    assert!(isolated
        .findings
        .iter()
        .any(|finding| finding.code == "isolated_worktree_merge_risk"));
    for node in &mut topology.nodes {
        for claim in &mut node.resource_claims {
            claim.mode = ResourceMode::Exclusive;
            claim.workspace_strategy = Some(WorkspaceStrategy::Shared);
        }
    }
    let report = lint_execution_topology(&topology);
    assert!(report.findings.iter().any(|finding| {
        finding.code == "unsafe_parallel_resource_conflict"
            && finding.classification == FindingClassification::Deterministic
            && finding.severity == LintSeverity::Error
    }));
}

#[test]
fn flat_thousand_node_fan_in_emits_context_pressure() {
    let base =
        fixture("tests/fixtures/casegraphen-design/independent-fanout/execution.topology.json");
    let mut source_template = base.nodes[0].clone();
    source_template.inputs.clear();
    source_template.outputs.clear();
    source_template.resource_claims.clear();
    source_template.verification_policy_id = None;
    source_template.delivery = DeliveryMode::Streaming;
    source_template.side_effects = SideEffects::None;
    let mut nodes = (0..1000)
        .map(|index| {
            let mut node = source_template.clone();
            node.node_id = format!("node:source-{index:04}");
            node.work_cell_id = format!("work:source-{index:04}");
            node.idempotency_key = format!("source-{index:04}");
            node
        })
        .collect::<Vec<_>>();
    let mut reduce = source_template;
    reduce.node_id = "node:reduce".to_owned();
    reduce.work_cell_id = "work:reduce".to_owned();
    reduce.idempotency_key = "reduce:<input-set-hash>".to_owned();
    reduce.delivery = DeliveryMode::Barrier;
    nodes.push(reduce);
    let edges = (0..1000)
        .map(|index| TopologyEdge {
            edge_id: format!("edge:source-{index:04}-reduce"),
            from: format!("node:source-{index:04}"),
            to: "node:reduce".to_owned(),
            kind: EdgeKind::Control,
            output: None,
            input: None,
            schema_id: None,
            blocking_predicate: format!("source {index} incomplete"),
            dependency_witness: "reduction waits for every declared source".to_owned(),
            removal_counterexample: "removal permits an incomplete reduction".to_owned(),
            resource_scope: vec![],
            provenance: Provenance {
                source: "test".to_owned(),
                created_by: "actor:test".to_owned(),
            },
        })
        .collect();
    let topology = ExecutionTopology {
        nodes,
        edges,
        topology_id: "topology:flat-1000".to_owned(),
        ..base
    };
    let report = lint_execution_topology(&topology);
    assert_eq!(report.metrics.maximum_fan_in, 1000);
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.code == "fan_in_context_pressure"));
}

#[test]
fn cycle_redundancy_and_heuristics_are_not_confused() {
    let mut topology = fixture("schemas/experimental/execution.topology.file-review.example.json");
    topology.edges.push(TopologyEdge {
        edge_id: "edge:verify-review-a".to_owned(),
        from: "node:verify".to_owned(),
        to: "node:review-a".to_owned(),
        kind: EdgeKind::Control,
        output: None,
        input: None,
        schema_id: None,
        blocking_predicate: "verification incomplete".to_owned(),
        dependency_witness: "cycle fixture".to_owned(),
        removal_counterexample: "removal breaks the fixture cycle".to_owned(),
        resource_scope: vec![],
        provenance: Provenance {
            source: "test".to_owned(),
            created_by: "actor:test".to_owned(),
        },
    });
    let report = lint_execution_topology(&topology);
    assert!(report.findings.iter().any(|finding| {
        finding.code == "dependency_cycle"
            && finding.classification == FindingClassification::Deterministic
    }));
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.classification == FindingClassification::Heuristic));
}

#[test]
fn edge_policy_barrier_authority_and_critical_path_checks_are_covered() {
    let mut topology = fixture("schemas/experimental/execution.topology.file-review.example.json");
    topology.edges[0].removal_counterexample = "no change".to_owned();
    topology.edges.push(TopologyEdge {
        edge_id: "edge:review-a-verify-redundant".to_owned(),
        from: "node:review-a".to_owned(),
        to: "node:verify".to_owned(),
        kind: EdgeKind::Data,
        output: Some("findings".to_owned()),
        input: Some("summary".to_owned()),
        schema_id: Some("schema:findings".to_owned()),
        blocking_predicate: "review A incomplete".to_owned(),
        dependency_witness: "direct reachability fixture".to_owned(),
        removal_counterexample: "review A could be bypassed".to_owned(),
        resource_scope: vec![],
        provenance: Provenance {
            source: "test".to_owned(),
            created_by: "actor:test".to_owned(),
        },
    });
    topology
        .expansion_policy_ids
        .push("expansion:test".to_owned());
    topology.nodes[0].expansion_policy_id = Some("expansion:test".to_owned());
    topology.nodes[0].budget_policy_id = None;
    let report = lint_execution_topology(&topology);
    assert_eq!(report.metrics.critical_path_ms, Some(1600));
    for code in [
        "removal_counterexample_no_change",
        "redundant_reachability",
        "false_edge_candidate",
        "barrier_on_pipeline_path",
        "verification_independence_uninspectable",
        "expansion_termination_uninspectable",
        "expansion_without_budget",
        "authority_concentration_candidate",
    ] {
        assert!(
            report.findings.iter().any(|finding| finding.code == code),
            "missing {code}: {:?}",
            report.findings
        );
    }
    assert!(report.findings.iter().any(|finding| {
        finding.code == "redundant_reachability"
            && finding.classification == FindingClassification::Deterministic
    }));
    assert!(report.findings.iter().any(|finding| {
        finding.code == "false_edge_candidate"
            && finding.classification == FindingClassification::Heuristic
    }));
}

#[test]
fn intrinsic_cross_reference_failures_use_the_central_validator() {
    let mut topology = fixture("schemas/experimental/execution.topology.file-review.example.json");
    topology.edges[0].to = "node:missing".to_owned();
    topology.edges[1].dependency_witness.clear();
    let report = lint_execution_topology(&topology);
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.code == "contract_unknown_edge_target"));
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.code == "contract_empty_required_field"));
}

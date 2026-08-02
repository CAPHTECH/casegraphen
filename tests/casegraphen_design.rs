#![allow(missing_docs)]

use casegraphen::execution_topology::{
    parse_execution_topology, EdgeKind, ExecutionTopology, ResourceMode,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> ExecutionTopology {
    let path = root().join(format!(
        "tests/fixtures/casegraphen-design/{name}/execution.topology.json"
    ));
    parse_execution_topology(&fs::read_to_string(&path).expect("read topology fixture"))
        .unwrap_or_else(|findings| panic!("{}: {findings:?}", path.display()))
}

fn expectation(name: &str) -> Value {
    let path = root().join(format!(
        "tests/fixtures/casegraphen-design/{name}/expected.behavior.json"
    ));
    serde_json::from_str(&fs::read_to_string(path).expect("read behavior expectation"))
        .expect("parse behavior expectation")
}

#[test]
fn every_fresh_context_fixture_is_a_valid_v0_proposal() {
    for entry in sorted_fixture_directories() {
        let topology_path = entry.join("execution.topology.json");
        let expectation_path = entry.join("expected.behavior.json");
        assert!(topology_path.is_file(), "{}", topology_path.display());
        assert!(expectation_path.is_file(), "{}", expectation_path.display());
        let topology = parse_execution_topology(
            &fs::read_to_string(&topology_path).expect("read fresh-context topology"),
        )
        .unwrap_or_else(|findings| panic!("{}: {findings:?}", topology_path.display()));
        let expected: Value = serde_json::from_str(
            &fs::read_to_string(&expectation_path).expect("read fresh-context expectation"),
        )
        .expect("parse fresh-context expectation");
        assert_eq!(expected["proposal_only"], true);
        assert_eq!(topology.schema_version, 0);
    }
}

#[test]
fn independent_files_preserve_twenty_way_fanout_without_fake_edges() {
    let topology = fixture("independent-fanout");
    let expected = expectation("independent-fanout");
    assert_eq!(
        topology.nodes.len(),
        expected["node_count"].as_u64().unwrap() as usize
    );
    assert_eq!(
        topology.edges.len(),
        expected["edge_count"].as_u64().unwrap() as usize
    );
    let resources = topology
        .nodes
        .iter()
        .flat_map(|node| node.resource_claims.iter())
        .map(|claim| claim.resource.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(resources.len(), 20);
}

#[test]
fn same_file_writers_preserve_the_hidden_resource_edge() {
    let topology = fixture("same-file-collision");
    let expected = expectation("same-file-collision");
    let resource = expected["required_resource_scope"].as_str().unwrap();
    assert!(topology.nodes.iter().all(|node| node
        .resource_claims
        .iter()
        .any(|claim| claim.resource == resource && claim.mode == ResourceMode::Write)));
    assert!(topology.edges.iter().any(|edge| {
        edge.kind == EdgeKind::ResourceExclusion
            && edge.resource_scope.iter().any(|scope| scope == resource)
    }));
}

#[test]
fn thousand_item_design_uses_bounded_hierarchical_reduction() {
    let topology = fixture("hierarchical-reduction");
    let expected = expectation("hierarchical-reduction");
    let mut indegree = BTreeMap::<&str, usize>::new();
    for edge in &topology.edges {
        *indegree.entry(edge.to.as_str()).or_default() += 1;
    }
    let reducers = topology
        .nodes
        .iter()
        .filter(|node| node.node_id.starts_with("node:reduce-"))
        .collect::<Vec<_>>();
    assert_eq!(
        reducers.len(),
        expected["reducer_count"].as_u64().unwrap() as usize
    );
    let max_reducer_fanin = reducers
        .iter()
        .map(|node| indegree.get(node.node_id.as_str()).copied().unwrap_or(0))
        .max()
        .unwrap();
    assert_eq!(
        max_reducer_fanin,
        expected["max_direct_fanin"].as_u64().unwrap() as usize
    );
    assert_eq!(
        indegree.get("node:synthesize").copied(),
        Some(expected["synthesis_fanin"].as_u64().unwrap() as usize)
    );
    assert_eq!(
        expected["batch_count"].as_u64().unwrap() * expected["items_per_batch"].as_u64().unwrap(),
        expected["source_item_count"].as_u64().unwrap()
    );
}

#[test]
fn correlated_verifier_metadata_is_preserved_for_the_real_linter() {
    let topology = fixture("verifier-correlation");
    let expected = expectation("verifier-correlation");
    assert_eq!(topology.nodes.len(), 2);
    assert!(topology.nodes.iter().all(|node| {
        node.executor_class == expected["shared_executor_class"].as_str().unwrap()
            && node.verification_policy_id.as_deref() == expected["verification_policy_id"].as_str()
    }));
    assert_eq!(expected["must_preserve_lint_warning"], true);
}

#[test]
fn stale_revision_basis_is_preserved_instead_of_rebased() {
    let expected = expectation("stale-revision");
    let mapping = fs::read_to_string(
        root().join("tests/fixtures/casegraphen-design/stale-revision/genesis.mapping.proposal.md"),
    )
    .expect("read stale mapping proposal");
    let observed = expected["observed_revision_id"].as_str().unwrap();
    let current = expected["current_revision_id"].as_str().unwrap();
    assert!(mapping.contains(&format!("observed_revision_id: `{observed}`")));
    assert!(mapping.contains(&format!("currently_reported_revision_id: `{current}`")));
    assert!(mapping.contains("applicability: stale basis"));
    assert!(mapping.contains("mutation performed: no"));
}

#[test]
fn executable_skill_surface_contains_only_the_read_only_linter() {
    let skill = fs::read_to_string(root().join("skills/casegraphen-design/SKILL.md"))
        .expect("read design Skill");
    let shell_commands = shell_casegraphen_commands(&skill);
    assert_eq!(
        shell_commands,
        vec!["casegraphen graph lint --input execution.topology.json --format json \\"]
    );
}

#[test]
fn fresh_context_proposals_run_through_the_shipped_linter_without_mutation() {
    let fixture_root = root().join("tests/fixtures/casegraphen-design");
    let before = directory_snapshot(&fixture_root);
    let output_root = TestOutputDirectory::new();
    let mut reports = BTreeMap::new();

    for entry in sorted_fixture_directories() {
        let name = entry
            .file_name()
            .and_then(|name| name.to_str())
            .expect("UTF-8 fixture name");
        let output_path = output_root.path.join(format!("{name}.report.json"));
        let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
            .args(["graph", "lint", "--input"])
            .arg(entry.join("execution.topology.json"))
            .args(["--format", "json", "--output"])
            .arg(&output_path)
            .output()
            .expect("run shipped graph linter");
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_str(
            &fs::read_to_string(&output_path).expect("read generated graph lint report"),
        )
        .expect("parse generated graph lint report");
        assert_eq!(
            report["schema"],
            "casegraphen.experimental.graph_lint.report.v0"
        );
        reports.insert(name.to_owned(), report);
    }

    assert_eq!(
        reports["independent-fanout"]["metrics"]["theoretical_parallel_width"],
        20
    );
    assert_eq!(
        reports["hierarchical-reduction"]["metrics"]["maximum_fan_in"],
        5
    );
    assert!(!finding_codes(&reports["hierarchical-reduction"]).contains(&"fan_in_context_pressure"));
    assert!(reports["same-file-collision"]["findings"]
        .as_array()
        .expect("same-file findings")
        .iter()
        .all(|finding| finding["code"] != "unsafe_parallel_resource_conflict"));
    assert!(reports["verifier-correlation"]["findings"]
        .as_array()
        .expect("verifier findings")
        .iter()
        .any(|finding| {
            finding["code"] == "verification_independence_uninspectable"
                && finding["classification"] == "heuristic"
        }));
    assert_eq!(before, directory_snapshot(&fixture_root));
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

fn sorted_fixture_directories() -> Vec<std::path::PathBuf> {
    let mut entries = fs::read_dir(root().join("tests/fixtures/casegraphen-design"))
        .expect("read casegraphen-design fixtures")
        .map(|entry| entry.expect("read fixture entry").path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn finding_codes(report: &Value) -> Vec<&str> {
    report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|finding| finding["code"].as_str().expect("finding code"))
        .collect()
}

fn directory_snapshot(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, current: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = fs::read_dir(current)
            .expect("read snapshot directory")
            .map(|entry| entry.expect("read snapshot entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                collect(root, &entry, files);
            } else {
                files.insert(
                    entry
                        .strip_prefix(root)
                        .expect("fixture under root")
                        .to_owned(),
                    fs::read(&entry).expect("read snapshot file"),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    collect(path, path, &mut files);
    files
}

struct TestOutputDirectory {
    path: PathBuf,
}

impl TestOutputDirectory {
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "casegraphen-design-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create test output directory");
        Self { path }
    }
}

impl Drop for TestOutputDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

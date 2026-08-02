//! Experimental execution-topology contract.
//!
//! This describes deployable graph shape without changing the stable case graph,
//! execution-plan, or runtime-report contracts. A parsed topology is a proposal;
//! validation and hashing do not make it accepted.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Experimental schema identity. It is intentionally outside the stable namespace.
pub const EXECUTION_TOPOLOGY_SCHEMA: &str = "casegraphen.experimental.execution.topology.v0";
/// Experimental schema version.
pub const EXECUTION_TOPOLOGY_SCHEMA_VERSION: u32 = 0;

/// A deployable topology proposal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionTopology {
    pub schema: String,
    pub schema_version: u32,
    pub topology_id: String,
    pub case_space_id: String,
    pub nodes: Vec<TopologyNode>,
    pub edges: Vec<TopologyEdge>,
    pub verification_policy_ids: Vec<String>,
    pub budget_policy_ids: Vec<String>,
    pub expansion_policy_ids: Vec<String>,
    pub completeness_policy: CompletenessPolicy,
    pub provenance: Provenance,
}

/// One runtime node mapped to governed work.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyNode {
    pub node_id: String,
    pub work_cell_id: String,
    pub purpose: String,
    pub inputs: Vec<NodeInput>,
    pub outputs: Vec<NodeOutput>,
    pub side_effects: SideEffects,
    pub resource_claims: Vec<ResourceClaim>,
    pub executor_class: String,
    pub verification_policy_id: Option<String>,
    pub budget_policy_id: Option<String>,
    pub idempotency_key: String,
    pub delivery: DeliveryMode,
    pub expansion_policy_id: Option<String>,
    pub estimated_duration_ms: Option<u64>,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeInput {
    pub name: String,
    pub schema_id: String,
    pub artifact_selector: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeOutput {
    pub name: String,
    pub schema_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffects {
    None,
    Workspace,
    External,
    WorkspaceAndExternal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceMode {
    Read,
    Write,
    Exclusive,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStrategy {
    Shared,
    IsolatedWorktree,
    Ephemeral,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceClaim {
    pub resource: String,
    pub mode: ResourceMode,
    pub rate_limit_group: Option<String>,
    pub workspace_strategy: Option<WorkspaceStrategy>,
    pub network_scope: Vec<String>,
    pub secret_scope: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Barrier,
    Streaming,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyEdge {
    pub edge_id: String,
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub output: Option<String>,
    pub input: Option<String>,
    pub schema_id: Option<String>,
    pub blocking_predicate: String,
    pub dependency_witness: String,
    pub removal_counterexample: String,
    pub resource_scope: Vec<String>,
    pub provenance: Provenance,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Data,
    Control,
    Evidence,
    ReviewOrAuthority,
    ResourceExclusion,
    Temporal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletenessPolicy {
    AllExpectedNodesReported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source: String,
    pub created_by: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyValidationFinding {
    pub code: String,
    pub location: String,
    pub detail: String,
}

/// Parses and applies all intrinsic semantic validation.
pub fn parse_execution_topology(
    input: &str,
) -> Result<ExecutionTopology, Vec<TopologyValidationFinding>> {
    let topology: ExecutionTopology = serde_json::from_str(input)
        .map_err(|error| vec![finding("invalid_json", "$", error.to_string())])?;
    let findings = validate_execution_topology(&topology);
    if findings.is_empty() {
        Ok(topology)
    } else {
        Err(findings)
    }
}

/// Validates topology-local identities, bindings, policies, and resource edges.
pub fn validate_execution_topology(topology: &ExecutionTopology) -> Vec<TopologyValidationFinding> {
    let mut findings = Vec::new();
    if topology.schema != EXECUTION_TOPOLOGY_SCHEMA {
        findings.push(finding(
            "unsupported_schema",
            "$.schema",
            "schema must name execution.topology.v0",
        ));
    }
    if topology.schema_version != EXECUTION_TOPOLOGY_SCHEMA_VERSION {
        findings.push(finding(
            "unsupported_schema_version",
            "$.schema_version",
            "schema_version must be 0",
        ));
    }
    required(&topology.topology_id, "$.topology_id", &mut findings);
    required(&topology.case_space_id, "$.case_space_id", &mut findings);
    if topology.nodes.is_empty() {
        findings.push(finding(
            "empty_topology",
            "$.nodes",
            "at least one node is required",
        ));
    }
    unique_names(
        topology.verification_policy_ids.iter().map(String::as_str),
        "duplicate_policy_id",
        "$.verification_policy_ids",
        &mut findings,
    );
    unique_names(
        topology.budget_policy_ids.iter().map(String::as_str),
        "duplicate_policy_id",
        "$.budget_policy_ids",
        &mut findings,
    );
    unique_names(
        topology.expansion_policy_ids.iter().map(String::as_str),
        "duplicate_policy_id",
        "$.expansion_policy_ids",
        &mut findings,
    );
    for (field, values) in [
        ("verification_policy_ids", &topology.verification_policy_ids),
        ("budget_policy_ids", &topology.budget_policy_ids),
        ("expansion_policy_ids", &topology.expansion_policy_ids),
    ] {
        for (index, value) in values.iter().enumerate() {
            required(value, &format!("$.{field}[{index}]"), &mut findings);
        }
    }
    required(
        &topology.provenance.source,
        "$.provenance.source",
        &mut findings,
    );
    required(
        &topology.provenance.created_by,
        "$.provenance.created_by",
        &mut findings,
    );

    let mut nodes = BTreeMap::new();
    for (index, node) in topology.nodes.iter().enumerate() {
        let location = format!("$.nodes[{index}]");
        for (field, value) in [
            ("node_id", node.node_id.as_str()),
            ("work_cell_id", node.work_cell_id.as_str()),
            ("purpose", node.purpose.as_str()),
            ("executor_class", node.executor_class.as_str()),
            ("idempotency_key", node.idempotency_key.as_str()),
        ] {
            required(value, &format!("{location}.{field}"), &mut findings);
        }
        if nodes.insert(node.node_id.as_str(), node).is_some() {
            findings.push(finding(
                "duplicate_node_id",
                format!("{location}.node_id"),
                "node_id must be unique",
            ));
        }
        unique_names(
            node.inputs.iter().map(|input| input.name.as_str()),
            "duplicate_input_name",
            &format!("{location}.inputs"),
            &mut findings,
        );
        unique_names(
            node.outputs.iter().map(|output| output.name.as_str()),
            "duplicate_output_name",
            &format!("{location}.outputs"),
            &mut findings,
        );
        for (input_index, input) in node.inputs.iter().enumerate() {
            for (field, value) in [
                ("name", input.name.as_str()),
                ("schema_id", input.schema_id.as_str()),
                ("artifact_selector", input.artifact_selector.as_str()),
            ] {
                required(
                    value,
                    &format!("{location}.inputs[{input_index}].{field}"),
                    &mut findings,
                );
            }
        }
        for (output_index, output) in node.outputs.iter().enumerate() {
            required(
                &output.name,
                &format!("{location}.outputs[{output_index}].name"),
                &mut findings,
            );
            required(
                &output.schema_id,
                &format!("{location}.outputs[{output_index}].schema_id"),
                &mut findings,
            );
        }
        for (claim_index, claim) in node.resource_claims.iter().enumerate() {
            required(
                &claim.resource,
                &format!("{location}.resource_claims[{claim_index}].resource"),
                &mut findings,
            );
            if let Some(group) = &claim.rate_limit_group {
                required(
                    group,
                    &format!("{location}.resource_claims[{claim_index}].rate_limit_group"),
                    &mut findings,
                );
            }
            for (scope_field, scopes) in [
                ("network_scope", &claim.network_scope),
                ("secret_scope", &claim.secret_scope),
            ] {
                for (scope_index, scope) in scopes.iter().enumerate() {
                    required(
                        scope,
                        &format!(
                            "{location}.resource_claims[{claim_index}].{scope_field}[{scope_index}]"
                        ),
                        &mut findings,
                    );
                }
            }
        }
        required(
            &node.provenance.source,
            &format!("{location}.provenance.source"),
            &mut findings,
        );
        required(
            &node.provenance.created_by,
            &format!("{location}.provenance.created_by"),
            &mut findings,
        );
        validate_policy_reference(
            node.verification_policy_id.as_deref(),
            &topology.verification_policy_ids,
            "verification_policy_id",
            &location,
            &mut findings,
        );
        validate_policy_reference(
            node.budget_policy_id.as_deref(),
            &topology.budget_policy_ids,
            "budget_policy_id",
            &location,
            &mut findings,
        );
        validate_policy_reference(
            node.expansion_policy_id.as_deref(),
            &topology.expansion_policy_ids,
            "expansion_policy_id",
            &location,
            &mut findings,
        );
    }

    let mut edge_ids = BTreeSet::new();
    for (index, edge) in topology.edges.iter().enumerate() {
        let location = format!("$.edges[{index}]");
        required(&edge.edge_id, &format!("{location}.edge_id"), &mut findings);
        required(&edge.from, &format!("{location}.from"), &mut findings);
        required(&edge.to, &format!("{location}.to"), &mut findings);
        required(
            &edge.blocking_predicate,
            &format!("{location}.blocking_predicate"),
            &mut findings,
        );
        required(
            &edge.dependency_witness,
            &format!("{location}.dependency_witness"),
            &mut findings,
        );
        required(
            &edge.removal_counterexample,
            &format!("{location}.removal_counterexample"),
            &mut findings,
        );
        required(
            &edge.provenance.source,
            &format!("{location}.provenance.source"),
            &mut findings,
        );
        required(
            &edge.provenance.created_by,
            &format!("{location}.provenance.created_by"),
            &mut findings,
        );
        for (scope_index, resource) in edge.resource_scope.iter().enumerate() {
            required(
                resource,
                &format!("{location}.resource_scope[{scope_index}]"),
                &mut findings,
            );
        }
        if !edge_ids.insert(edge.edge_id.as_str()) {
            findings.push(finding(
                "duplicate_edge_id",
                format!("{location}.edge_id"),
                "edge_id must be unique",
            ));
        }
        let source = nodes.get(edge.from.as_str()).copied();
        let target = nodes.get(edge.to.as_str()).copied();
        if source.is_none() {
            findings.push(finding(
                "unknown_edge_source",
                format!("{location}.from"),
                "from must name a topology node",
            ));
        }
        if target.is_none() {
            findings.push(finding(
                "unknown_edge_target",
                format!("{location}.to"),
                "to must name a topology node",
            ));
        }
        if edge.from == edge.to {
            findings.push(finding(
                "self_edge",
                location.clone(),
                "an execution dependency cannot target its own node",
            ));
        }
        match edge.kind {
            EdgeKind::Data => validate_data_edge(edge, source, target, &location, &mut findings),
            _ if edge.output.is_some() || edge.input.is_some() || edge.schema_id.is_some() => {
                findings.push(finding(
                    "binding_on_non_data_edge",
                    location.clone(),
                    "only data edges carry output/input/schema bindings",
                ));
            }
            EdgeKind::ResourceExclusion if edge.resource_scope.is_empty() => {
                findings.push(finding(
                    "missing_resource_scope",
                    format!("{location}.resource_scope"),
                    "resource_exclusion requires at least one protected resource",
                ))
            }
            EdgeKind::ResourceExclusion => {
                if let (Some(source), Some(target)) = (source, target) {
                    for resource in &edge.resource_scope {
                        let source_claims = source
                            .resource_claims
                            .iter()
                            .any(|claim| claim.resource == *resource);
                        let target_claims = target
                            .resource_claims
                            .iter()
                            .any(|claim| claim.resource == *resource);
                        if !source_claims || !target_claims {
                            findings.push(finding(
                                "unknown_resource_scope",
                                format!("{location}.resource_scope"),
                                format!(
                                    "{resource} must be declared by both endpoints of a resource_exclusion edge"
                                ),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    sort_findings(&mut findings);
    findings
}

/// Validates external work-cell references without merging case semantics into v0.
pub fn validate_work_cell_references(
    topology: &ExecutionTopology,
    work_cell_ids: &BTreeSet<String>,
) -> Vec<TopologyValidationFinding> {
    let mut findings = topology
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| !work_cell_ids.contains(&node.work_cell_id))
        .map(|(index, node)| {
            finding(
                "unknown_work_cell",
                format!("$.nodes[{index}].work_cell_id"),
                format!(
                    "{} is not present in the supplied case graph",
                    node.work_cell_id
                ),
            )
        })
        .collect::<Vec<_>>();
    sort_findings(&mut findings);
    findings
}

/// Serializes a normalized topology. Array order is not an authority-bearing distinction.
pub fn canonical_execution_topology(
    topology: &ExecutionTopology,
) -> Result<String, serde_json::Error> {
    let mut normalized = topology.clone();
    normalized
        .nodes
        .sort_by(|left, right| left.node_id.cmp(&right.node_id));
    normalized
        .edges
        .sort_by(|left, right| left.edge_id.cmp(&right.edge_id));
    normalized.verification_policy_ids.sort();
    normalized.budget_policy_ids.sort();
    normalized.expansion_policy_ids.sort();
    for node in &mut normalized.nodes {
        node.inputs
            .sort_by(|left, right| left.name.cmp(&right.name));
        node.outputs
            .sort_by(|left, right| left.name.cmp(&right.name));
        for claim in &mut node.resource_claims {
            claim.network_scope.sort();
            claim.secret_scope.sort();
        }
        node.resource_claims.sort();
    }
    for edge in &mut normalized.edges {
        edge.resource_scope.sort();
    }
    serde_json::to_string(&normalized)
}

/// Hashes the canonical execution topology.
pub fn execution_topology_content_hash(
    topology: &ExecutionTopology,
) -> Result<String, serde_json::Error> {
    let canonical = canonical_execution_topology(topology)?;
    Ok(crate::native_hash::sha256_hex(canonical.as_bytes()))
}

fn validate_data_edge(
    edge: &TopologyEdge,
    source: Option<&TopologyNode>,
    target: Option<&TopologyNode>,
    location: &str,
    findings: &mut Vec<TopologyValidationFinding>,
) {
    let (Some(output), Some(input), Some(schema_id)) = (
        edge.output.as_deref(),
        edge.input.as_deref(),
        edge.schema_id.as_deref(),
    ) else {
        findings.push(finding(
            "incomplete_data_binding",
            location,
            "data edges require output, input, and schema_id",
        ));
        return;
    };
    if let Some(source) = source {
        if !source
            .outputs
            .iter()
            .any(|candidate| candidate.name == output && candidate.schema_id == schema_id)
        {
            findings.push(finding(
                "unknown_output_binding",
                format!("{location}.output"),
                "output and schema_id must match a source output",
            ));
        }
    }
    if let Some(target) = target {
        if !target
            .inputs
            .iter()
            .any(|candidate| candidate.name == input && candidate.schema_id == schema_id)
        {
            findings.push(finding(
                "unknown_input_binding",
                format!("{location}.input"),
                "input and schema_id must match a target input",
            ));
        }
    }
}

fn validate_policy_reference(
    reference: Option<&str>,
    known: &[String],
    field: &str,
    location: &str,
    findings: &mut Vec<TopologyValidationFinding>,
) {
    if let Some(reference) = reference {
        if !known.iter().any(|candidate| candidate == reference) {
            findings.push(finding(
                "unknown_policy_reference",
                format!("{location}.{field}"),
                format!("{reference} is not declared by this topology"),
            ));
        }
    }
}

fn unique_names<'a>(
    values: impl Iterator<Item = &'a str>,
    code: &str,
    location: &str,
    findings: &mut Vec<TopologyValidationFinding>,
) {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            findings.push(finding(code, location, format!("duplicate name {value:?}")));
        }
    }
}

fn required(value: &str, location: &str, findings: &mut Vec<TopologyValidationFinding>) {
    if value.trim().is_empty() {
        findings.push(finding(
            "empty_required_field",
            location,
            "value must not be empty",
        ));
    }
}

fn finding(
    code: impl Into<String>,
    location: impl Into<String>,
    detail: impl Into<String>,
) -> TopologyValidationFinding {
    TopologyValidationFinding {
        code: code.into(),
        location: location.into(),
        detail: detail.into(),
    }
}

fn sort_findings(findings: &mut [TopologyValidationFinding]) {
    findings.sort_by(|left, right| {
        (&left.code, &left.location, &left.detail).cmp(&(
            &right.code,
            &right.location,
            &right.detail,
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(name: &str) -> ExecutionTopology {
        let path = format!("{}/schemas/experimental/{name}", env!("CARGO_MANIFEST_DIR"));
        parse_execution_topology(&std::fs::read_to_string(path).expect("read example"))
            .expect("valid topology example")
    }

    #[test]
    fn both_runtime_designs_validate() {
        example("execution.topology.file-review.example.json");
        example("execution.topology.worktree.example.json");
        let hierarchical = format!(
            "{}/tests/fixtures/casegraphen-design/hierarchical-reduction/execution.topology.json",
            env!("CARGO_MANIFEST_DIR")
        );
        parse_execution_topology(
            &std::fs::read_to_string(hierarchical).expect("read hierarchical example"),
        )
        .expect("valid hierarchical reduction example");
    }

    #[test]
    fn canonical_hash_ignores_manifest_array_order() {
        let mut left = example("execution.topology.file-review.example.json");
        let mut right = left.clone();
        right.nodes.reverse();
        right.edges.reverse();
        right.verification_policy_ids.reverse();
        assert_eq!(
            execution_topology_content_hash(&left).expect("hash left"),
            execution_topology_content_hash(&right).expect("hash right")
        );
        left.nodes[0].purpose.push_str(" changed");
        assert_ne!(
            execution_topology_content_hash(&left).expect("hash changed"),
            execution_topology_content_hash(&right).expect("hash original")
        );
    }

    #[test]
    fn unknown_fields_and_cross_references_fail_closed() {
        let text = std::fs::read_to_string(format!(
            "{}/schemas/experimental/execution.topology.file-review.example.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("read example");
        let mut value: serde_json::Value = serde_json::from_str(&text).expect("parse value");
        value["unexpected"] = serde_json::json!(true);
        assert_eq!(
            parse_execution_topology(&value.to_string()).unwrap_err()[0].code,
            "invalid_json"
        );

        let mut omitted: serde_json::Value = serde_json::from_str(&text).expect("parse value");
        omitted
            .as_object_mut()
            .expect("topology object")
            .remove("budget_policy_ids");
        assert_eq!(
            parse_execution_topology(&omitted.to_string()).unwrap_err()[0].code,
            "invalid_json"
        );

        let mut topology = example("execution.topology.file-review.example.json");
        topology.edges[0].to = "node:missing".to_owned();
        assert!(validate_execution_topology(&topology)
            .iter()
            .any(|finding| finding.code == "unknown_edge_target"));

        topology.nodes[0].inputs[0].schema_id.clear();
        assert!(validate_execution_topology(&topology)
            .iter()
            .any(|finding| {
                finding.code == "empty_required_field"
                    && finding.location == "$.nodes[0].inputs[0].schema_id"
            }));

        let mut work_cells = BTreeSet::new();
        work_cells.insert("work:review-a".to_owned());
        assert!(validate_work_cell_references(&topology, &work_cells)
            .iter()
            .any(|finding| finding.code == "unknown_work_cell"));
    }

    #[test]
    fn shipped_schema_is_json_and_names_the_experimental_contract() {
        let text = std::fs::read_to_string(format!(
            "{}/schemas/experimental/execution.topology.v0.schema.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("read schema");
        let schema: serde_json::Value = serde_json::from_str(&text).expect("schema JSON");
        assert_eq!(schema["$id"], EXECUTION_TOPOLOGY_SCHEMA);
        assert_eq!(schema["additionalProperties"], false);
    }
}

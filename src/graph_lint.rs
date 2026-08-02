//! Deterministic analysis of an experimental [`ExecutionTopology`].
//!
//! This module consumes the typed contract.  It deliberately does not parse a
//! second JSON graph representation and does not reproduce case readiness,
//! evidence, review, or authorization decisions.

use crate::execution_topology::{
    execution_topology_content_hash, validate_execution_topology, CompletenessPolicy, DeliveryMode,
    EdgeKind, ExecutionTopology, ResourceMode, SideEffects, WorkspaceStrategy,
};
use crate::verification_policy::{validate_verification_policy, VerificationPolicy};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const GRAPH_LINT_REPORT_SCHEMA: &str = "casegraphen.experimental.graph_lint.report.v0";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingClassification {
    Deterministic,
    Heuristic,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LintSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GraphLintFinding {
    pub code: String,
    pub classification: FindingClassification,
    pub severity: LintSeverity,
    pub location: String,
    pub detail: String,
    pub suggested_next_operation: SuggestedOperation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SuggestedOperation {
    pub operation: String,
    pub target_id: Option<String>,
    pub parameters: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GraphLintMetrics {
    pub node_count: usize,
    pub edge_count: usize,
    pub source_count: usize,
    pub sink_count: usize,
    pub theoretical_parallel_width: usize,
    pub longest_path_nodes: Option<usize>,
    pub critical_path_ms: Option<u64>,
    pub maximum_fan_in: usize,
    pub maximum_fan_out: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GraphLintReport {
    pub schema: &'static str,
    pub report_version: u32,
    pub topology_id: String,
    pub topology_content_hash: String,
    pub metrics: GraphLintMetrics,
    pub findings: Vec<GraphLintFinding>,
}

impl GraphLintReport {
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }
}

/// Analyze graph shape. The caller must first apply the topology contract's
/// intrinsic validation; keeping that boundary explicit prevents two validators
/// for the same rule from drifting.
pub fn lint_execution_topology(topology: &ExecutionTopology) -> GraphLintReport {
    let node_index = topology
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.node_id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut adjacency = vec![Vec::new(); topology.nodes.len()];
    let mut reverse = vec![Vec::new(); topology.nodes.len()];
    for (edge_index, edge) in topology.edges.iter().enumerate() {
        if let (Some(&from), Some(&to)) = (
            node_index.get(edge.from.as_str()),
            node_index.get(edge.to.as_str()),
        ) {
            adjacency[from].push((to, edge_index));
            reverse[to].push((from, edge_index));
        }
    }
    for edges in &mut adjacency {
        edges.sort_by_key(|(node, edge)| (*node, *edge));
    }
    for edges in &mut reverse {
        edges.sort_by_key(|(node, edge)| (*node, *edge));
    }

    let (acyclic, width, longest) = dag_metrics(&adjacency, &reverse);
    let mut findings = validate_execution_topology(topology)
        .into_iter()
        .map(|validation| {
            finding(
                format!("contract_{}", validation.code),
                FindingClassification::Deterministic,
                LintSeverity::Error,
                validation.location,
                validation.detail,
            )
        })
        .collect::<Vec<_>>();
    if !acyclic {
        findings.push(finding(
            "dependency_cycle",
            FindingClassification::Deterministic,
            LintSeverity::Error,
            "$.edges",
            "the topology contains a dependency cycle",
        ));
    }

    // An alternative path is a deterministic structural fact. Whether the
    // direct edge is semantically fake is intentionally only a heuristic.
    for (edge_index, edge) in topology.edges.iter().enumerate() {
        let (Some(&from), Some(&to)) = (
            node_index.get(edge.from.as_str()),
            node_index.get(edge.to.as_str()),
        ) else {
            continue;
        };
        if reachable_ignoring_edge_of_kind(
            from,
            to,
            edge_index,
            edge.kind,
            &adjacency,
            &topology.edges,
        ) {
            findings.push(finding(
                "redundant_reachability",
                FindingClassification::Deterministic,
                LintSeverity::Info,
                format!("$.edges[{edge_index}]"),
                format!(
                    "{} still reaches {} when {} is removed",
                    edge.from, edge.to, edge.edge_id
                ),
            ));
            findings.push(finding(
                "false_edge_candidate",
                FindingClassification::Heuristic,
                LintSeverity::Warning,
                format!("$.edges[{edge_index}]"),
                "the direct edge may be unnecessary; review its witness and removal counterexample",
            ));
        }
        if matches!(
            edge.removal_counterexample
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "none" | "no change" | "not applicable" | "n/a"
        ) {
            findings.push(finding(
                "removal_counterexample_no_change",
                FindingClassification::Deterministic,
                LintSeverity::Warning,
                format!("$.edges[{edge_index}].removal_counterexample"),
                "the supplied counterexample says removing the edge changes nothing",
            ));
        }
    }

    resource_conflicts(topology, &node_index, &adjacency, &mut findings);

    for (index, node) in topology.nodes.iter().enumerate() {
        if node.delivery == DeliveryMode::Barrier
            && !reverse[index].is_empty()
            && !adjacency[index].is_empty()
        {
            findings.push(finding(
                "barrier_on_pipeline_path",
                FindingClassification::Heuristic,
                LintSeverity::Warning,
                format!("$.nodes[{index}].delivery"),
                "a barrier node lies between upstream and downstream work",
            ));
        }
        if reverse[index].len() >= 100 {
            findings.push(finding(
                "fan_in_context_pressure",
                FindingClassification::Heuristic,
                LintSeverity::Warning,
                format!("$.nodes[{index}]"),
                format!(
                    "node has {} direct predecessors; consider hierarchical reduction",
                    reverse[index].len()
                ),
            ));
        }
        if node.side_effects != SideEffects::None && node.verification_policy_id.is_none() {
            findings.push(finding(
                "side_effect_without_verification_policy",
                FindingClassification::Heuristic,
                LintSeverity::Warning,
                format!("$.nodes[{index}].verification_policy_id"),
                "a side-effecting node has no declared verification policy",
            ));
        }
        if node.expansion_policy_id.is_some() && node.budget_policy_id.is_none() {
            findings.push(finding(
                "expansion_without_budget",
                FindingClassification::Deterministic,
                LintSeverity::Error,
                format!("$.nodes[{index}].budget_policy_id"),
                "an expanding node must reference a budget policy",
            ));
        }
    }

    for policy_id in &topology.verification_policy_ids {
        let affected = topology
            .nodes
            .iter()
            .filter(|node| node.verification_policy_id.as_ref() == Some(policy_id))
            .count();
        findings.push(finding(
            "verification_independence_uninspectable",
            FindingClassification::Heuristic,
            LintSeverity::Info,
            "$.verification_policy_ids",
            format!(
                "policy {policy_id} governs {affected} node(s), but v0 does not embed verifier or world-anchor constraints"
            ),
        ));
    }
    for policy_id in &topology.expansion_policy_ids {
        let affected = topology
            .nodes
            .iter()
            .filter(|node| node.expansion_policy_id.as_ref() == Some(policy_id))
            .count();
        findings.push(finding(
            "expansion_termination_uninspectable",
            FindingClassification::Heuristic,
            LintSeverity::Info,
            "$.expansion_policy_ids",
            format!(
                "policy {policy_id} governs {affected} node(s), but v0 cannot inspect its termination bounds"
            ),
        ));
    }

    if topology.completeness_policy != CompletenessPolicy::AllExpectedNodesReported {
        // Kept explicit for forward-compatible enum growth.
        findings.push(finding(
            "incomplete_reporting_policy",
            FindingClassification::Deterministic,
            LintSeverity::Error,
            "$.completeness_policy",
            "topology must require all expected nodes to report",
        ));
    }

    let governed = topology
        .nodes
        .iter()
        .filter(|node| node.verification_policy_id.is_some())
        .collect::<Vec<_>>();
    if governed.len() > 1
        && governed
            .iter()
            .map(|node| node.provenance.created_by.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            == 1
    {
        findings.push(finding(
            "authority_concentration_candidate",
            FindingClassification::Heuristic,
            LintSeverity::Warning,
            "$.nodes",
            "all verification-governed nodes were proposed by the same actor; this does not prove verifier correlation",
        ));
    }

    findings.sort_by(|left, right| {
        (
            &left.classification,
            &left.severity,
            &left.code,
            &left.location,
            &left.detail,
        )
            .cmp(&(
                &right.classification,
                &right.severity,
                &right.code,
                &right.location,
                &right.detail,
            ))
    });
    GraphLintReport {
        schema: GRAPH_LINT_REPORT_SCHEMA,
        report_version: 0,
        topology_id: topology.topology_id.clone(),
        topology_content_hash: execution_topology_content_hash(topology)
            .expect("typed execution topology serializes"),
        metrics: GraphLintMetrics {
            node_count: topology.nodes.len(),
            edge_count: topology.edges.len(),
            source_count: reverse.iter().filter(|edges| edges.is_empty()).count(),
            sink_count: adjacency.iter().filter(|edges| edges.is_empty()).count(),
            theoretical_parallel_width: width,
            longest_path_nodes: longest,
            critical_path_ms: acyclic
                .then(|| critical_path_ms(topology, &reverse))
                .flatten(),
            maximum_fan_in: reverse.iter().map(Vec::len).max().unwrap_or(0),
            maximum_fan_out: adjacency.iter().map(Vec::len).max().unwrap_or(0),
        },
        findings,
    }
}

/// Enrich graph-shape lint with the actual verification policy documents.
/// Runtime/ledger observations remain the verification reconciler's concern;
/// this function only replaces an "uninspectable" warning with deterministic
/// policy presence/shape checks and explicit static independence risks.
pub fn lint_execution_topology_with_verification_policies(
    topology: &ExecutionTopology,
    policies: &BTreeMap<String, VerificationPolicy>,
) -> GraphLintReport {
    let mut report = lint_execution_topology(topology);
    report
        .findings
        .retain(|finding| finding.code != "verification_independence_uninspectable");
    for policy_id in &topology.verification_policy_ids {
        let Some(policy) = policies.get(policy_id) else {
            report.findings.push(finding(
                "verification_policy_missing",
                FindingClassification::Deterministic,
                LintSeverity::Error,
                "$.verification_policy_ids",
                format!("topology references {policy_id}, but no policy document was supplied"),
            ));
            continue;
        };
        for violation in validate_verification_policy(policy) {
            report.findings.push(finding(
                format!("verification_policy_{}", violation.code),
                FindingClassification::Deterministic,
                LintSeverity::Error,
                "$.verification_policy_ids",
                violation.detail,
            ));
        }
        if policy.verification_policy_id != *policy_id {
            report.findings.push(finding(
                "verification_policy_identity_mismatch",
                FindingClassification::Deterministic,
                LintSeverity::Error,
                "$.verification_policy_ids",
                format!("map key {policy_id} differs from the policy document identity"),
            ));
        }
        if !policy.actor_must_differ {
            report.findings.push(finding(
                "verifier_actor_correlation_allowed",
                FindingClassification::Heuristic,
                LintSeverity::Warning,
                "$.verification_policy_ids",
                format!(
                    "policy {policy_id} permits producer and verifier actor identity to coincide"
                ),
            ));
        }
        if policy.required_anchors.is_empty() {
            report.findings.push(finding(
                "verification_anchor_missing",
                FindingClassification::Heuristic,
                LintSeverity::Warning,
                "$.verification_policy_ids",
                format!("policy {policy_id} requires no world anchor"),
            ));
        }
        if !policy.allowed_runtime_attestations.is_empty() {
            report.findings.push(finding(
                "runtime_attestation_not_independence_proof",
                FindingClassification::Heuristic,
                LintSeverity::Info,
                "$.verification_policy_ids",
                format!("policy {policy_id} allows runtime attestations, which cannot prove fresh context or independent minds"),
            ));
        }
    }
    report.findings.sort_by(|left, right| {
        (
            &left.classification,
            &left.severity,
            &left.code,
            &left.location,
            &left.detail,
        )
            .cmp(&(
                &right.classification,
                &right.severity,
                &right.code,
                &right.location,
                &right.detail,
            ))
    });
    report
}

/// Human-readable projection of the same typed report.
pub fn render_graph_lint_text(report: &GraphLintReport) -> String {
    let mut lines = vec![
        format!("topology: {}", report.topology_id),
        format!("content hash: {}", report.topology_content_hash),
        format!(
            "nodes: {}, edges: {}, parallel width: {}, critical path ms: {}",
            report.metrics.node_count,
            report.metrics.edge_count,
            report.metrics.theoretical_parallel_width,
            report
                .metrics
                .critical_path_ms
                .map_or_else(|| "unknown".to_owned(), |value| value.to_string())
        ),
        format!("findings: {}", report.findings.len()),
    ];
    for finding in &report.findings {
        lines.push(format!(
            "- [{:?}/{:?}] {} {}: {}",
            finding.classification,
            finding.severity,
            finding.code,
            finding.location,
            finding.detail
        ));
    }
    lines.join("\n")
}

fn critical_path_ms(topology: &ExecutionTopology, reverse: &[Vec<(usize, usize)>]) -> Option<u64> {
    let durations = topology
        .nodes
        .iter()
        .map(|node| node.estimated_duration_ms)
        .collect::<Option<Vec<_>>>()?;
    let mut indegree = reverse.iter().map(Vec::len).collect::<Vec<_>>();
    let mut successors = vec![Vec::new(); topology.nodes.len()];
    for (to, predecessors) in reverse.iter().enumerate() {
        for &(from, _) in predecessors {
            successors[from].push(to);
        }
    }
    let mut queue = indegree
        .iter()
        .enumerate()
        .filter_map(|(index, &degree)| (degree == 0).then_some(index))
        .collect::<VecDeque<_>>();
    let mut longest = durations.clone();
    while let Some(node) = queue.pop_front() {
        for &next in &successors[node] {
            longest[next] = longest[next].max(longest[node].saturating_add(durations[next]));
            indegree[next] -= 1;
            if indegree[next] == 0 {
                queue.push_back(next);
            }
        }
    }
    longest.into_iter().max()
}

fn dag_metrics(
    adjacency: &[Vec<(usize, usize)>],
    reverse: &[Vec<(usize, usize)>],
) -> (bool, usize, Option<usize>) {
    let mut indegree = reverse.iter().map(Vec::len).collect::<Vec<_>>();
    let mut queue = indegree
        .iter()
        .enumerate()
        .filter_map(|(i, &d)| (d == 0).then_some(i))
        .collect::<VecDeque<_>>();
    let mut processed = 0;
    let mut maximum_width = queue.len();
    let mut distance = vec![1usize; adjacency.len()];
    while !queue.is_empty() {
        let level = queue.len();
        maximum_width = maximum_width.max(level);
        for _ in 0..level {
            let node = queue.pop_front().expect("level length checked");
            processed += 1;
            for &(next, _) in &adjacency[node] {
                distance[next] = distance[next].max(distance[node] + 1);
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    queue.push_back(next);
                }
            }
        }
    }
    let acyclic = processed == adjacency.len();
    (
        acyclic,
        maximum_width,
        acyclic.then(|| distance.into_iter().max().unwrap_or(0)),
    )
}

fn reachable_ignoring_edge(
    from: usize,
    to: usize,
    ignored: usize,
    adjacency: &[Vec<(usize, usize)>],
) -> bool {
    let mut seen = vec![false; adjacency.len()];
    let mut queue = VecDeque::from([from]);
    seen[from] = true;
    while let Some(node) = queue.pop_front() {
        for &(next, edge) in &adjacency[node] {
            if edge == ignored {
                continue;
            }
            if next == to {
                return true;
            }
            if !seen[next] {
                seen[next] = true;
                queue.push_back(next);
            }
        }
    }
    false
}

fn reachable_ignoring_edge_of_kind(
    from: usize,
    to: usize,
    ignored: usize,
    kind: EdgeKind,
    adjacency: &[Vec<(usize, usize)>],
    edges: &[crate::execution_topology::TopologyEdge],
) -> bool {
    let mut seen = vec![false; adjacency.len()];
    let mut queue = VecDeque::from([from]);
    seen[from] = true;
    while let Some(node) = queue.pop_front() {
        for &(next, edge_index) in &adjacency[node] {
            if edge_index == ignored || edges[edge_index].kind != kind {
                continue;
            }
            if next == to {
                return true;
            }
            if !seen[next] {
                seen[next] = true;
                queue.push_back(next);
            }
        }
    }
    false
}

fn resource_conflicts(
    topology: &ExecutionTopology,
    index: &BTreeMap<&str, usize>,
    adjacency: &[Vec<(usize, usize)>],
    findings: &mut Vec<GraphLintFinding>,
) {
    for left in 0..topology.nodes.len() {
        for right in (left + 1)..topology.nodes.len() {
            if reachable_ignoring_edge(left, right, usize::MAX, adjacency)
                || reachable_ignoring_edge(right, left, usize::MAX, adjacency)
            {
                continue;
            }
            for l in &topology.nodes[left].resource_claims {
                for r in &topology.nodes[right].resource_claims {
                    if l.resource != r.resource || !claims_conflict(l.mode, r.mode) {
                        continue;
                    }
                    let isolated_file = l.resource.starts_with("file:")
                        && l.workspace_strategy == Some(WorkspaceStrategy::IsolatedWorktree)
                        && r.workspace_strategy == Some(WorkspaceStrategy::IsolatedWorktree);
                    if isolated_file {
                        findings.push(finding(
                            "isolated_worktree_merge_risk",
                            FindingClassification::Heuristic,
                            LintSeverity::Info,
                            format!("$.nodes[{left}],$.nodes[{right}]"),
                            format!(
                                "isolated worktrees prevent direct writes to {} from colliding, but integration may still conflict",
                                l.resource
                            ),
                        ));
                        continue;
                    }
                    let protected = topology.edges.iter().any(|edge| {
                        edge.kind == EdgeKind::ResourceExclusion
                            && edge.resource_scope.contains(&l.resource)
                            && ((index.get(edge.from.as_str()) == Some(&left)
                                && index.get(edge.to.as_str()) == Some(&right))
                                || (index.get(edge.from.as_str()) == Some(&right)
                                    && index.get(edge.to.as_str()) == Some(&left)))
                    });
                    if !protected {
                        findings.push(finding(
                            "unsafe_parallel_resource_conflict",
                            FindingClassification::Deterministic,
                            LintSeverity::Error,
                            format!("$.nodes[{left}],$.nodes[{right}]"),
                            format!("unordered nodes claim conflicting access to {}", l.resource),
                        ));
                    }
                }
            }
        }
    }
}

fn claims_conflict(left: ResourceMode, right: ResourceMode) -> bool {
    left != ResourceMode::Read || right != ResourceMode::Read
}

fn finding(
    code: impl Into<String>,
    classification: FindingClassification,
    severity: LintSeverity,
    location: impl Into<String>,
    detail: impl Into<String>,
) -> GraphLintFinding {
    let code = code.into();
    GraphLintFinding {
        suggested_next_operation: SuggestedOperation {
            operation: suggested_operation(&code).to_owned(),
            target_id: None,
            parameters: BTreeMap::new(),
        },
        code,
        classification,
        severity,
        location: location.into(),
        detail: detail.into(),
    }
}

fn suggested_operation(code: &str) -> &'static str {
    match code {
        "dependency_cycle" => "remove_or_reclassify_edge",
        "redundant_reachability" | "false_edge_candidate" | "removal_counterexample_no_change" => {
            "review_edge_removal"
        }
        "unsafe_parallel_resource_conflict" => "add_resource_exclusion",
        "fan_in_context_pressure" => "propose_hierarchical_reduction",
        "barrier_on_pipeline_path" => "review_barrier_necessity",
        "expansion_without_budget" => "attach_budget_policy",
        code if code.starts_with("contract_") => "repair_topology_contract",
        _ => "review_finding",
    }
}

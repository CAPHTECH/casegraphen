//! Deterministic, bounded simulation of experimental execution topologies.
//!
//! Simulation consumes a topology and calibration input by reference. It emits
//! diagnostics and unreviewed routing proposals; it cannot mutate or accept the
//! topology, a plan, or CaseGraphen state.

use crate::{
    execution_topology::{
        execution_topology_content_hash, ExecutionTopology, ResourceMode, TopologyNode,
    },
    graph_lint::{lint_execution_topology, LintSeverity},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const GRAPH_SIMULATION_REQUEST_SCHEMA: &str =
    "casegraphen.experimental.graph_simulation.request.v0";
pub const GRAPH_SIMULATION_REPORT_SCHEMA: &str =
    "casegraphen.experimental.graph_simulation.report.v0";
pub const GRAPH_SIMULATION_MAX_ITERATIONS: u32 = 10_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct U64Range {
    pub minimum: u64,
    pub maximum: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeCalibration {
    pub node_id: Option<String>,
    pub executor_class: Option<String>,
    pub latency_ms: Option<U64Range>,
    pub cost_microunits: Option<U64Range>,
    pub failure_basis_points: Option<u16>,
    pub input_tokens: Option<U64Range>,
    pub output_tokens: Option<U64Range>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingCandidate {
    pub route_id: String,
    pub executor_class: String,
    pub latency_ms: U64Range,
    pub cost_microunits: U64Range,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSimulationRequest {
    pub schema: String,
    pub schema_version: u32,
    pub topology_content_hash: String,
    pub seed: u64,
    pub iterations: u32,
    pub max_parallelism: u32,
    pub resource_capacities: BTreeMap<String, u32>,
    pub fan_in_penalty_ms_per_input: u64,
    pub streaming_overlap_basis_points: Option<u16>,
    pub retry_policy: RetryPolicy,
    pub expansion_bounds: BTreeMap<String, U64Range>,
    pub budgets: SimulationBudgets,
    pub calibrations: Vec<NodeCalibration>,
    pub routing_candidates: Vec<RoutingCandidate>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    pub maximum_attempts: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationBudgets {
    pub maximum_latency_ms: Option<u64>,
    pub maximum_cost_microunits: Option<u64>,
    pub maximum_total_tokens: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SimulationFinding {
    pub code: String,
    pub location: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct SimulationUnknown {
    pub node_id: String,
    pub metric: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SimulationRange {
    pub minimum: u64,
    pub p50: u64,
    pub maximum: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RoutingDecisionProposal {
    pub node_id: String,
    pub current_executor_class: String,
    pub proposed_route_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RoutingProposal {
    pub review_status: &'static str,
    pub topology_content_hash: String,
    pub decisions: Vec<RoutingDecisionProposal>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GraphSimulationReport {
    pub schema: &'static str,
    pub report_version: u32,
    pub topology_id: String,
    pub topology_content_hash: String,
    pub request_content_hash: String,
    pub seed: u64,
    pub iterations: u32,
    pub latency_ms: Option<SimulationRange>,
    pub cost_microunits: Option<SimulationRange>,
    pub total_tokens: Option<SimulationRange>,
    pub successful_iterations: Option<u32>,
    pub failed_iterations: Option<u32>,
    pub maximum_observed_parallelism: u32,
    pub resource_wait_events: u64,
    pub unknowns: Vec<SimulationUnknown>,
    pub budget_violations: Vec<BudgetViolation>,
    pub routing_proposal: RoutingProposal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BudgetViolation {
    pub budget: String,
    pub violating_iterations: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GraphSimulationComparison {
    pub review_status: &'static str,
    pub baseline_topology_hash: String,
    pub candidate_topology_hash: String,
    pub latency_p50_delta_ms: Option<i64>,
    pub cost_p50_delta_microunits: Option<i64>,
    pub token_p50_delta: Option<i64>,
    pub unknowns: Vec<String>,
}

pub fn compare_simulation_reports(
    baseline: &GraphSimulationReport,
    candidate: &GraphSimulationReport,
) -> GraphSimulationComparison {
    let mut unknowns = Vec::new();
    let latency = range_delta(baseline.latency_ms.as_ref(), candidate.latency_ms.as_ref());
    let cost = range_delta(
        baseline.cost_microunits.as_ref(),
        candidate.cost_microunits.as_ref(),
    );
    let tokens = range_delta(
        baseline.total_tokens.as_ref(),
        candidate.total_tokens.as_ref(),
    );
    if latency.is_none() {
        unknowns.push("latency comparison unavailable".to_owned());
    }
    if cost.is_none() {
        unknowns.push("cost comparison unavailable".to_owned());
    }
    if tokens.is_none() {
        unknowns.push("token comparison unavailable".to_owned());
    }
    GraphSimulationComparison {
        review_status: "unreviewed",
        baseline_topology_hash: baseline.topology_content_hash.clone(),
        candidate_topology_hash: candidate.topology_content_hash.clone(),
        latency_p50_delta_ms: latency,
        cost_p50_delta_microunits: cost,
        token_p50_delta: tokens,
        unknowns,
    }
}

pub fn parse_graph_simulation_request(
    input: &str,
) -> Result<GraphSimulationRequest, Vec<SimulationFinding>> {
    let request: GraphSimulationRequest = serde_json::from_str(input)
        .map_err(|error| vec![finding("invalid_json", "$", error.to_string())])?;
    let findings = validate_request(&request);
    if findings.is_empty() {
        Ok(request)
    } else {
        Err(findings)
    }
}

pub fn simulate_execution_topology(
    topology: &ExecutionTopology,
    request: &GraphSimulationRequest,
) -> Result<GraphSimulationReport, Vec<SimulationFinding>> {
    let mut findings = validate_request(request);
    let topology_hash = execution_topology_content_hash(topology)
        .expect("typed execution topology serializes deterministically");
    if request.topology_content_hash != topology_hash {
        findings.push(finding(
            "topology_hash_mismatch",
            "$.topology_content_hash",
            "calibration input must name the exact topology content hash",
        ));
    }
    let lint = lint_execution_topology(topology);
    findings.extend(
        lint.findings
            .iter()
            .filter(|finding| finding.severity == LintSeverity::Error)
            .map(|lint| {
                finding(
                    format!("lint_{}", lint.code),
                    lint.location.clone(),
                    lint.detail.clone(),
                )
            }),
    );
    if !findings.is_empty() {
        sort_findings(&mut findings);
        return Err(findings);
    }

    let graph = SimulationGraph::new(topology);
    let mut unknowns = BTreeSet::new();
    let calibrations = resolve_calibrations(topology, request, &mut unknowns);
    let latency_known = calibrations
        .iter()
        .all(|calibration| calibration.latency.is_some());
    let cost_known = calibrations
        .iter()
        .all(|calibration| calibration.cost.is_some());
    let tokens_known = calibrations
        .iter()
        .all(|calibration| calibration.tokens.is_some());
    let failure_known = calibrations
        .iter()
        .all(|calibration| calibration.failure_basis_points.is_some());
    let mut latency_samples = Vec::new();
    let mut cost_samples = Vec::new();
    let mut token_samples = Vec::new();
    let mut failed_iterations = 0_u32;
    let mut maximum_parallelism = 0;
    let mut wait_events = 0_u64;
    let mut rng = SeededRng::new(request.seed);

    for _ in 0..request.iterations {
        let sampled = calibrations
            .iter()
            .enumerate()
            .map(|(index, calibration)| {
                let expansion = topology.nodes[index]
                    .expansion_policy_id
                    .as_ref()
                    .and_then(|policy| request.expansion_bounds.get(policy))
                    .map(|range| rng.sample(*range))
                    .unwrap_or(1)
                    .max(1);
                let mut attempts = 1_u64;
                let mut failed = false;
                if let Some(probability) = calibration.failure_basis_points {
                    while rng.next() % 10_000 < u64::from(probability) {
                        if attempts >= u64::from(request.retry_policy.maximum_attempts) {
                            failed = true;
                            break;
                        }
                        attempts += 1;
                    }
                }
                let latency = calibration
                    .latency
                    .map(|range| rng.sample(range))
                    .unwrap_or(0)
                    .saturating_mul(attempts)
                    .saturating_mul(expansion);
                let cost = calibration.cost.map(|range| {
                    rng.sample(range)
                        .saturating_mul(attempts)
                        .saturating_mul(expansion)
                });
                let tokens = calibration.tokens.map(|(input, output)| {
                    rng.sample(input)
                        .saturating_add(rng.sample(output))
                        .saturating_mul(attempts)
                        .saturating_mul(expansion)
                });
                SampledNode {
                    latency_ms: latency,
                    cost_microunits: cost,
                    total_tokens: tokens,
                    failed,
                }
            })
            .collect::<Vec<_>>();
        if latency_known {
            let run = graph.simulate(topology, request, &sampled);
            latency_samples.push(run.elapsed_ms);
            maximum_parallelism = maximum_parallelism.max(run.maximum_parallelism);
            wait_events = wait_events.saturating_add(run.resource_wait_events);
        }
        if cost_known {
            cost_samples.push(
                sampled
                    .iter()
                    .map(|node| node.cost_microunits.expect("cost_known"))
                    .fold(0_u64, u64::saturating_add),
            );
        }
        if tokens_known {
            token_samples.push(
                sampled
                    .iter()
                    .map(|node| node.total_tokens.expect("tokens_known"))
                    .fold(0_u64, u64::saturating_add),
            );
        }
        if failure_known && sampled.iter().any(|node| node.failed) {
            failed_iterations += 1;
        }
    }
    let request_content_hash = crate::native_hash::sha256_hex(
        serde_json::to_string(request)
            .expect("typed request serializes")
            .as_bytes(),
    );
    Ok(GraphSimulationReport {
        schema: GRAPH_SIMULATION_REPORT_SCHEMA,
        report_version: 0,
        topology_id: topology.topology_id.clone(),
        topology_content_hash: topology_hash.clone(),
        request_content_hash,
        seed: request.seed,
        iterations: request.iterations,
        latency_ms: summarize(&latency_samples),
        cost_microunits: summarize(&cost_samples),
        total_tokens: summarize(&token_samples),
        successful_iterations: failure_known.then_some(request.iterations - failed_iterations),
        failed_iterations: failure_known.then_some(failed_iterations),
        maximum_observed_parallelism: maximum_parallelism,
        resource_wait_events: wait_events,
        unknowns: unknowns.into_iter().collect(),
        budget_violations: budget_violations(
            request,
            &latency_samples,
            &cost_samples,
            &token_samples,
        ),
        routing_proposal: routing_proposal(topology, request, topology_hash),
    })
}

fn validate_request(request: &GraphSimulationRequest) -> Vec<SimulationFinding> {
    let mut findings = Vec::new();
    if request.schema != GRAPH_SIMULATION_REQUEST_SCHEMA {
        findings.push(finding(
            "unsupported_schema",
            "$.schema",
            "schema must name graph_simulation.request.v0",
        ));
    }
    if request.schema_version != 0 {
        findings.push(finding(
            "unsupported_schema_version",
            "$.schema_version",
            "schema_version must be 0",
        ));
    }
    if request.iterations == 0 || request.iterations > GRAPH_SIMULATION_MAX_ITERATIONS {
        findings.push(finding(
            "iterations_out_of_bounds",
            "$.iterations",
            format!("iterations must be between 1 and {GRAPH_SIMULATION_MAX_ITERATIONS}"),
        ));
    }
    if request.max_parallelism == 0 {
        findings.push(finding(
            "zero_parallelism",
            "$.max_parallelism",
            "max_parallelism must be greater than zero",
        ));
    }
    if request.retry_policy.maximum_attempts == 0 || request.retry_policy.maximum_attempts > 100 {
        findings.push(finding(
            "retry_attempts_out_of_bounds",
            "$.retry_policy.maximum_attempts",
            "maximum_attempts must be between 1 and 100",
        ));
    }
    if request
        .streaming_overlap_basis_points
        .is_some_and(|value| value > 10_000)
    {
        findings.push(finding(
            "invalid_streaming_overlap",
            "$.streaming_overlap_basis_points",
            "basis points must not exceed 10000",
        ));
    }
    for (resource, capacity) in &request.resource_capacities {
        if resource.trim().is_empty() || *capacity == 0 {
            findings.push(finding(
                "invalid_resource_capacity",
                "$.resource_capacities",
                "resource ids must be non-empty and capacities must be positive",
            ));
        }
    }
    for (policy, range) in &request.expansion_bounds {
        if policy.trim().is_empty() {
            findings.push(finding(
                "empty_expansion_policy",
                "$.expansion_bounds",
                "expansion policy id must not be empty",
            ));
        }
        validate_range(Some(*range), "$.expansion_bounds", &mut findings);
    }
    let mut calibration_selectors = BTreeSet::new();
    for (index, calibration) in request.calibrations.iter().enumerate() {
        if calibration.node_id.is_none() == calibration.executor_class.is_none() {
            findings.push(finding(
                "ambiguous_calibration_selector",
                format!("$.calibrations[{index}]"),
                "exactly one of node_id or executor_class is required",
            ));
        }
        if let Some(selector) = calibration
            .node_id
            .as_ref()
            .map(|id| format!("node:{id}"))
            .or_else(|| {
                calibration
                    .executor_class
                    .as_ref()
                    .map(|class| format!("class:{class}"))
            })
        {
            if !calibration_selectors.insert(selector) {
                findings.push(finding(
                    "duplicate_calibration_selector",
                    format!("$.calibrations[{index}]"),
                    "each node or executor class may have only one calibration",
                ));
            }
        }
        validate_range(
            calibration.latency_ms,
            &format!("$.calibrations[{index}].latency_ms"),
            &mut findings,
        );
        if calibration
            .failure_basis_points
            .is_some_and(|probability| probability > 10_000)
        {
            findings.push(finding(
                "invalid_failure_probability",
                format!("$.calibrations[{index}].failure_basis_points"),
                "basis points must not exceed 10000",
            ));
        }
        validate_range(
            calibration.input_tokens,
            &format!("$.calibrations[{index}].input_tokens"),
            &mut findings,
        );
        validate_range(
            calibration.output_tokens,
            &format!("$.calibrations[{index}].output_tokens"),
            &mut findings,
        );
        validate_range(
            calibration.cost_microunits,
            &format!("$.calibrations[{index}].cost_microunits"),
            &mut findings,
        );
    }
    let mut route_ids = BTreeSet::new();
    for (index, route) in request.routing_candidates.iter().enumerate() {
        if route.route_id.trim().is_empty() || route.executor_class.trim().is_empty() {
            findings.push(finding(
                "empty_routing_candidate",
                format!("$.routing_candidates[{index}]"),
                "route_id and executor_class must be non-empty",
            ));
        }
        if !route_ids.insert(&route.route_id) {
            findings.push(finding(
                "duplicate_route_id",
                format!("$.routing_candidates[{index}].route_id"),
                "route_id must be unique",
            ));
        }
        validate_range(
            Some(route.latency_ms),
            &format!("$.routing_candidates[{index}].latency_ms"),
            &mut findings,
        );
        validate_range(
            Some(route.cost_microunits),
            &format!("$.routing_candidates[{index}].cost_microunits"),
            &mut findings,
        );
    }
    sort_findings(&mut findings);
    findings
}

fn validate_range(range: Option<U64Range>, location: &str, findings: &mut Vec<SimulationFinding>) {
    if range.is_some_and(|range| range.minimum > range.maximum) {
        findings.push(finding(
            "invalid_range",
            location,
            "minimum must not exceed maximum",
        ));
    }
}

#[derive(Clone, Copy)]
struct ResolvedCalibration {
    latency: Option<U64Range>,
    cost: Option<U64Range>,
    failure_basis_points: Option<u16>,
    tokens: Option<(U64Range, U64Range)>,
}

fn resolve_calibrations(
    topology: &ExecutionTopology,
    request: &GraphSimulationRequest,
    unknowns: &mut BTreeSet<SimulationUnknown>,
) -> Vec<ResolvedCalibration> {
    topology
        .nodes
        .iter()
        .map(|node| {
            let specific = request
                .calibrations
                .iter()
                .find(|calibration| calibration.node_id.as_deref() == Some(&node.node_id));
            let class = request.calibrations.iter().find(|calibration| {
                calibration.executor_class.as_deref() == Some(&node.executor_class)
            });
            let latency = specific
                .and_then(|calibration| calibration.latency_ms)
                .or_else(|| class.and_then(|calibration| calibration.latency_ms))
                .or_else(|| {
                    node.estimated_duration_ms.map(|duration| U64Range {
                        minimum: duration,
                        maximum: duration,
                    })
                });
            let cost = specific
                .and_then(|calibration| calibration.cost_microunits)
                .or_else(|| class.and_then(|calibration| calibration.cost_microunits));
            let failure_basis_points = specific
                .and_then(|calibration| calibration.failure_basis_points)
                .or_else(|| class.and_then(|calibration| calibration.failure_basis_points));
            let input_tokens = specific
                .and_then(|calibration| calibration.input_tokens)
                .or_else(|| class.and_then(|calibration| calibration.input_tokens));
            let output_tokens = specific
                .and_then(|calibration| calibration.output_tokens)
                .or_else(|| class.and_then(|calibration| calibration.output_tokens));
            if latency.is_none() {
                unknowns.insert(SimulationUnknown {
                    node_id: node.node_id.clone(),
                    metric: "latency_ms".to_owned(),
                    reason: "no node/class calibration or topology duration estimate".to_owned(),
                });
            }
            if cost.is_none() {
                unknowns.insert(SimulationUnknown {
                    node_id: node.node_id.clone(),
                    metric: "cost_microunits".to_owned(),
                    reason: "no node or executor-class cost calibration".to_owned(),
                });
            }
            if failure_basis_points.is_none() {
                unknowns.insert(SimulationUnknown {
                    node_id: node.node_id.clone(),
                    metric: "failure_probability".to_owned(),
                    reason: "no node or executor-class failure calibration".to_owned(),
                });
            }
            if input_tokens.is_none() || output_tokens.is_none() {
                unknowns.insert(SimulationUnknown {
                    node_id: node.node_id.clone(),
                    metric: "token_envelope".to_owned(),
                    reason: "input and output token ranges are both required".to_owned(),
                });
            }
            if node.delivery == crate::execution_topology::DeliveryMode::Streaming {
                unknowns.insert(SimulationUnknown {
                    node_id: node.node_id.clone(),
                    metric: "streaming_partial_release".to_owned(),
                    reason: "v0 conservatively schedules streaming nodes as barriers; overlap input is recorded but does not claim partial-release precision".to_owned(),
                });
            }
            if node
                .expansion_policy_id
                .as_ref()
                .is_some_and(|policy| !request.expansion_bounds.contains_key(policy))
            {
                unknowns.insert(SimulationUnknown {
                    node_id: node.node_id.clone(),
                    metric: "expansion_bound".to_owned(),
                    reason: "expansion policy has no bounded spawned-node calibration".to_owned(),
                });
            }
            ResolvedCalibration {
                latency,
                cost,
                failure_basis_points,
                tokens: input_tokens.zip(output_tokens),
            }
        })
        .collect()
}

struct SimulationGraph {
    successors: Vec<Vec<usize>>,
    indegree: Vec<usize>,
    fan_in: Vec<usize>,
}

struct SampledNode {
    latency_ms: u64,
    cost_microunits: Option<u64>,
    total_tokens: Option<u64>,
    failed: bool,
}

struct RunResult {
    elapsed_ms: u64,
    maximum_parallelism: u32,
    resource_wait_events: u64,
}

impl SimulationGraph {
    fn new(topology: &ExecutionTopology) -> Self {
        let indices = topology
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.node_id.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        let mut successors = vec![Vec::new(); topology.nodes.len()];
        let mut indegree = vec![0; topology.nodes.len()];
        for edge in &topology.edges {
            if let (Some(&from), Some(&to)) = (
                indices.get(edge.from.as_str()),
                indices.get(edge.to.as_str()),
            ) {
                successors[from].push(to);
                indegree[to] += 1;
            }
        }
        for list in &mut successors {
            list.sort_unstable();
        }
        Self {
            successors,
            fan_in: indegree.clone(),
            indegree,
        }
    }

    fn simulate(
        &self,
        topology: &ExecutionTopology,
        request: &GraphSimulationRequest,
        sampled: &[SampledNode],
    ) -> RunResult {
        let mut indegree = self.indegree.clone();
        let mut ready = indegree
            .iter()
            .enumerate()
            .filter_map(|(index, &degree)| (degree == 0).then_some(index))
            .collect::<BTreeSet<_>>();
        let mut active = Vec::<(u64, usize, Vec<String>)>::new();
        let mut usage = BTreeMap::<String, u32>::new();
        let mut now = 0_u64;
        let mut completed = 0;
        let mut maximum_parallelism = 0;
        let mut waits = 0_u64;
        while completed < topology.nodes.len() {
            let candidates = ready.iter().copied().collect::<Vec<_>>();
            for node_index in candidates {
                if active.len() >= request.max_parallelism as usize {
                    break;
                }
                let resources = resource_keys(&topology.nodes[node_index]);
                if resources.iter().any(|resource| {
                    usage.get(resource).copied().unwrap_or(0)
                        >= resource_capacity(resource, &topology.nodes[node_index], request)
                }) {
                    waits = waits.saturating_add(1);
                    continue;
                }
                ready.remove(&node_index);
                for resource in &resources {
                    *usage.entry(resource.clone()).or_default() += 1;
                }
                let penalty = request
                    .fan_in_penalty_ms_per_input
                    .saturating_mul(self.fan_in[node_index] as u64);
                active.push((
                    now.saturating_add(sampled[node_index].latency_ms.saturating_add(penalty)),
                    node_index,
                    resources,
                ));
                maximum_parallelism = maximum_parallelism.max(active.len() as u32);
            }
            let next = active
                .iter()
                .map(|(finish, _, _)| *finish)
                .min()
                .expect("valid acyclic graph with positive parallelism always makes progress");
            now = next;
            let mut finished = active
                .iter()
                .enumerate()
                .filter_map(|(position, (finish, _, _))| (*finish == next).then_some(position))
                .collect::<Vec<_>>();
            finished.reverse();
            for position in finished {
                let (_, node, resources) = active.swap_remove(position);
                for resource in resources {
                    let count = usage.get_mut(&resource).expect("reserved resource");
                    *count -= 1;
                }
                completed += 1;
                for &successor in &self.successors[node] {
                    indegree[successor] -= 1;
                    if indegree[successor] == 0 {
                        ready.insert(successor);
                    }
                }
            }
        }
        RunResult {
            elapsed_ms: now,
            maximum_parallelism,
            resource_wait_events: waits,
        }
    }
}

fn resource_keys(node: &TopologyNode) -> Vec<String> {
    let mut keys = BTreeSet::new();
    for claim in &node.resource_claims {
        if claim.mode != ResourceMode::Read {
            keys.insert(claim.resource.clone());
        }
        if let Some(group) = &claim.rate_limit_group {
            keys.insert(format!("rate_limit_group:{group}"));
        }
    }
    keys.into_iter().collect()
}

fn resource_capacity(
    resource: &str,
    _node: &TopologyNode,
    request: &GraphSimulationRequest,
) -> u32 {
    request
        .resource_capacities
        .get(resource)
        .copied()
        .unwrap_or(1)
}

fn routing_proposal(
    topology: &ExecutionTopology,
    request: &GraphSimulationRequest,
    topology_hash: String,
) -> RoutingProposal {
    let mut decisions = topology
        .nodes
        .iter()
        .filter_map(|node| {
            let candidate = request
                .routing_candidates
                .iter()
                .filter(|candidate| candidate.executor_class == node.executor_class)
                .min_by_key(|candidate| {
                    (
                        midpoint(candidate.cost_microunits),
                        midpoint(candidate.latency_ms),
                        candidate.route_id.as_str(),
                    )
                })?;
            Some(RoutingDecisionProposal {
                node_id: node.node_id.clone(),
                current_executor_class: node.executor_class.clone(),
                proposed_route_id: candidate.route_id.clone(),
                reason:
                    "lowest midpoint cost; latency midpoint and route id are deterministic ties"
                        .to_owned(),
            })
        })
        .collect::<Vec<_>>();
    decisions.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    RoutingProposal {
        review_status: "unreviewed",
        topology_content_hash: topology_hash,
        decisions,
    }
}

fn midpoint(range: U64Range) -> u64 {
    range
        .minimum
        .saturating_add((range.maximum - range.minimum) / 2)
}

fn summarize(samples: &[u64]) -> Option<SimulationRange> {
    if samples.is_empty() {
        return None;
    }
    let mut samples = samples.to_vec();
    samples.sort_unstable();
    Some(SimulationRange {
        minimum: samples[0],
        p50: samples[(samples.len() - 1) / 2],
        maximum: *samples.last().expect("not empty"),
    })
}

fn range_delta(
    baseline: Option<&SimulationRange>,
    candidate: Option<&SimulationRange>,
) -> Option<i64> {
    let baseline = baseline?.p50;
    let candidate = candidate?.p50;
    Some((candidate as i128 - baseline as i128).clamp(i64::MIN as i128, i64::MAX as i128) as i64)
}

fn budget_violations(
    request: &GraphSimulationRequest,
    latency: &[u64],
    cost: &[u64],
    tokens: &[u64],
) -> Vec<BudgetViolation> {
    let mut violations = Vec::new();
    for (name, maximum, samples) in [
        ("latency_ms", request.budgets.maximum_latency_ms, latency),
        (
            "cost_microunits",
            request.budgets.maximum_cost_microunits,
            cost,
        ),
        ("total_tokens", request.budgets.maximum_total_tokens, tokens),
    ] {
        if let Some(maximum) = maximum {
            violations.push(BudgetViolation {
                budget: name.to_owned(),
                violating_iterations: samples.iter().filter(|&&value| value > maximum).count()
                    as u32,
            });
        }
    }
    violations
}

struct SeededRng(u64);

impl SeededRng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn sample(&mut self, range: U64Range) -> u64 {
        let width = range.maximum.saturating_sub(range.minimum);
        if width == 0 {
            range.minimum
        } else {
            range
                .minimum
                .saturating_add(self.next() % width.saturating_add(1))
        }
    }
}

fn finding(
    code: impl Into<String>,
    location: impl Into<String>,
    detail: impl Into<String>,
) -> SimulationFinding {
    SimulationFinding {
        code: code.into(),
        location: location.into(),
        detail: detail.into(),
    }
}

fn sort_findings(findings: &mut [SimulationFinding]) {
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
    use crate::execution_topology::{
        parse_execution_topology, CompletenessPolicy, DeliveryMode, EdgeKind, NodeInput,
        NodeOutput, Provenance, ResourceClaim, SideEffects, TopologyEdge, TopologyNode,
        EXECUTION_TOPOLOGY_SCHEMA,
    };

    fn example() -> ExecutionTopology {
        parse_execution_topology(include_str!(
            "../schemas/experimental/execution.topology.file-review.example.json"
        ))
        .unwrap()
    }

    fn request(topology: &ExecutionTopology) -> GraphSimulationRequest {
        let classes = topology
            .nodes
            .iter()
            .map(|node| node.executor_class.clone())
            .collect::<BTreeSet<_>>();
        GraphSimulationRequest {
            schema: GRAPH_SIMULATION_REQUEST_SCHEMA.to_owned(),
            schema_version: 0,
            topology_content_hash: execution_topology_content_hash(topology).unwrap(),
            seed: 55,
            iterations: 20,
            max_parallelism: 2_000,
            resource_capacities: BTreeMap::new(),
            fan_in_penalty_ms_per_input: 0,
            streaming_overlap_basis_points: Some(0),
            retry_policy: RetryPolicy {
                maximum_attempts: 3,
            },
            expansion_bounds: BTreeMap::new(),
            budgets: SimulationBudgets {
                maximum_latency_ms: Some(1_000_000),
                maximum_cost_microunits: Some(1_000_000),
                maximum_total_tokens: Some(1_000_000),
            },
            calibrations: classes
                .into_iter()
                .map(|class| NodeCalibration {
                    node_id: None,
                    executor_class: Some(class),
                    latency_ms: Some(U64Range {
                        minimum: 1,
                        maximum: 5,
                    }),
                    cost_microunits: Some(U64Range {
                        minimum: 2,
                        maximum: 4,
                    }),
                    failure_basis_points: Some(500),
                    input_tokens: Some(U64Range {
                        minimum: 10,
                        maximum: 20,
                    }),
                    output_tokens: Some(U64Range {
                        minimum: 2,
                        maximum: 5,
                    }),
                })
                .collect(),
            routing_candidates: vec![RoutingCandidate {
                route_id: "route:cheap-review".to_owned(),
                executor_class: "llm-reviewer".to_owned(),
                latency_ms: U64Range {
                    minimum: 2,
                    maximum: 3,
                },
                cost_microunits: U64Range {
                    minimum: 1,
                    maximum: 1,
                },
            }],
        }
    }

    #[test]
    fn seeded_simulation_is_deterministic_bounded_and_does_not_mutate_topology() {
        let topology = example();
        let before = execution_topology_content_hash(&topology).unwrap();
        let request = request(&topology);
        let first = simulate_execution_topology(&topology, &request).unwrap();
        let second = simulate_execution_topology(&topology, &request).unwrap();
        assert_eq!(first, second);
        assert_eq!(before, execution_topology_content_hash(&topology).unwrap());
        assert_eq!(first.iterations, 20);
        assert_eq!(first.routing_proposal.review_status, "unreviewed");
        assert!(!first.routing_proposal.decisions.is_empty());
    }

    #[test]
    fn request_example_parses_and_iteration_bound_fails_closed() {
        let mut request = parse_graph_simulation_request(include_str!(
            "../schemas/experimental/graph_simulation.request.example.json"
        ))
        .unwrap();
        request.iterations = GRAPH_SIMULATION_MAX_ITERATIONS + 1;
        assert!(validate_request(&request)
            .iter()
            .any(|finding| finding.code == "iterations_out_of_bounds"));
    }

    #[test]
    fn absent_calibration_is_an_explicit_unknown_not_a_zero_estimate() {
        let topology = example();
        let mut request = request(&topology);
        request.calibrations.clear();
        let report = simulate_execution_topology(&topology, &request).unwrap();
        assert!(report.cost_microunits.is_none());
        assert!(report.total_tokens.is_none());
        assert!(report.failed_iterations.is_none());
        assert!(report
            .unknowns
            .iter()
            .any(|unknown| unknown.metric == "cost_microunits"));
    }

    #[test]
    fn seeded_failures_apply_bounded_retries_and_report_budget_violations() {
        let topology = example();
        let mut request = request(&topology);
        request.iterations = 3;
        request.retry_policy.maximum_attempts = 3;
        request.budgets.maximum_cost_microunits = Some(1);
        for calibration in &mut request.calibrations {
            calibration.latency_ms = Some(U64Range {
                minimum: 10,
                maximum: 10,
            });
            calibration.cost_microunits = Some(U64Range {
                minimum: 2,
                maximum: 2,
            });
            calibration.failure_basis_points = Some(10_000);
        }
        let report = simulate_execution_topology(&topology, &request).unwrap();
        assert_eq!(report.failed_iterations, Some(3));
        assert_eq!(report.successful_iterations, Some(0));
        assert!(report
            .budget_violations
            .iter()
            .any(|violation| violation.budget == "cost_microunits"
                && violation.violating_iterations == 3));
    }

    #[test]
    fn rate_limit_capacity_reduces_parallelism_and_records_waiting() {
        let mut topology = example();
        for node in topology
            .nodes
            .iter_mut()
            .filter(|node| node.executor_class == "llm-reviewer")
        {
            node.resource_claims[0].rate_limit_group = Some("review-api".to_owned());
        }
        let mut request = request(&topology);
        request.topology_content_hash = execution_topology_content_hash(&topology).unwrap();
        request
            .resource_capacities
            .insert("rate_limit_group:review-api".to_owned(), 1);
        let report = simulate_execution_topology(&topology, &request).unwrap();
        assert!(report.resource_wait_events > 0);
        assert!(report.maximum_observed_parallelism <= 2);
    }

    #[test]
    fn deterministic_latency_agrees_with_linter_critical_path() {
        let topology = example();
        let mut request = request(&topology);
        request.iterations = 1;
        request.calibrations.clear();
        request.retry_policy.maximum_attempts = 1;
        let report = simulate_execution_topology(&topology, &request).unwrap();
        let lint = lint_execution_topology(&topology);
        assert_eq!(
            report.latency_ms.unwrap().p50,
            lint.metrics.critical_path_ms.unwrap()
        );
    }

    #[test]
    fn hierarchical_reduction_beats_flat_fan_in_for_one_thousand_inputs() {
        let flat = synthetic_reduction(1_000, None);
        let hierarchical = synthetic_reduction(1_000, Some(10));
        let mut flat_request = request(&flat);
        flat_request.iterations = 1;
        flat_request.fan_in_penalty_ms_per_input = 1;
        for calibration in &mut flat_request.calibrations {
            calibration.latency_ms = Some(U64Range {
                minimum: 1,
                maximum: 1,
            });
            calibration.failure_basis_points = Some(0);
        }
        let mut hierarchical_request = flat_request.clone();
        hierarchical_request.topology_content_hash =
            execution_topology_content_hash(&hierarchical).unwrap();
        let flat_report = simulate_execution_topology(&flat, &flat_request).unwrap();
        let hierarchical_report =
            simulate_execution_topology(&hierarchical, &hierarchical_request).unwrap();
        let comparison = compare_simulation_reports(&flat_report, &hierarchical_report);
        assert!(comparison.latency_p50_delta_ms.unwrap() < 0);
        assert_eq!(comparison.review_status, "unreviewed");
    }

    fn synthetic_reduction(count: usize, group_size: Option<usize>) -> ExecutionTopology {
        let provenance = Provenance {
            source: "test:synthetic".to_owned(),
            created_by: "actor:test".to_owned(),
        };
        let mut nodes = (0..count)
            .map(|index| node(&format!("node:source:{index}"), "source", &provenance))
            .collect::<Vec<_>>();
        let mut edges = Vec::new();
        match group_size {
            None => {
                nodes.push(node("node:final", "reducer", &provenance));
                for index in 0..count {
                    edges.push(edge(
                        &format!("edge:source:{index}:final"),
                        &format!("node:source:{index}"),
                        "node:final",
                        &provenance,
                    ));
                }
            }
            Some(group_size) => {
                let groups = count.div_ceil(group_size);
                for group in 0..groups {
                    nodes.push(node(&format!("node:group:{group}"), "reducer", &provenance));
                }
                nodes.push(node("node:final", "reducer", &provenance));
                for index in 0..count {
                    let group = index / group_size;
                    edges.push(edge(
                        &format!("edge:source:{index}:group:{group}"),
                        &format!("node:source:{index}"),
                        &format!("node:group:{group}"),
                        &provenance,
                    ));
                }
                for group in 0..groups {
                    edges.push(edge(
                        &format!("edge:group:{group}:final"),
                        &format!("node:group:{group}"),
                        "node:final",
                        &provenance,
                    ));
                }
            }
        }
        ExecutionTopology {
            schema: EXECUTION_TOPOLOGY_SCHEMA.to_owned(),
            schema_version: 0,
            topology_id: format!("topology:synthetic:{count}:{group_size:?}"),
            case_space_id: "case_space:simulation".to_owned(),
            nodes,
            edges,
            verification_policy_ids: Vec::new(),
            budget_policy_ids: Vec::new(),
            expansion_policy_ids: Vec::new(),
            completeness_policy: CompletenessPolicy::AllExpectedNodesReported,
            provenance,
        }
    }

    fn node(id: &str, class: &str, provenance: &Provenance) -> TopologyNode {
        TopologyNode {
            node_id: id.to_owned(),
            work_cell_id: format!("work:{id}"),
            purpose: "simulation fixture".to_owned(),
            inputs: vec![NodeInput {
                name: "input".to_owned(),
                schema_id: "schema:item".to_owned(),
                artifact_selector: "synthetic".to_owned(),
            }],
            outputs: vec![NodeOutput {
                name: "output".to_owned(),
                schema_id: "schema:item".to_owned(),
            }],
            side_effects: SideEffects::None,
            resource_claims: Vec::<ResourceClaim>::new(),
            executor_class: class.to_owned(),
            verification_policy_id: None,
            budget_policy_id: None,
            idempotency_key: format!("idempotency:{id}"),
            delivery: DeliveryMode::Barrier,
            expansion_policy_id: None,
            estimated_duration_ms: Some(1),
            provenance: provenance.clone(),
        }
    }

    fn edge(id: &str, from: &str, to: &str, provenance: &Provenance) -> TopologyEdge {
        TopologyEdge {
            edge_id: id.to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            kind: EdgeKind::Data,
            output: Some("output".to_owned()),
            input: Some("input".to_owned()),
            schema_id: Some("schema:item".to_owned()),
            blocking_predicate: "input absent".to_owned(),
            dependency_witness: "output binds input".to_owned(),
            removal_counterexample: "input would be omitted".to_owned(),
            resource_scope: Vec::new(),
            provenance: provenance.clone(),
        }
    }
}

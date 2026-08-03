//! Bounded dynamic-topology discovery producing reviewable proposals only.

use crate::{
    execution_topology::{
        execution_topology_content_hash, validate_execution_topology, ExecutionTopology,
        Provenance, TopologyEdge, TopologyNode,
    },
    graph_compiler::CompilationMode,
    graph_lint::{lint_execution_topology, FindingClassification, LintSeverity},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const EXPANSION_POLICY_SCHEMA: &str = "casegraphen.experimental.expansion.policy.v0";
pub const TOPOLOGY_PATCH_SCHEMA: &str = "casegraphen.experimental.topology.patch.v0";
pub const TOPOLOGY_PATCH_SCHEMA_VERSION: u32 = 0;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DedupeScope {
    AllSeen,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionPolicy {
    pub schema: String,
    pub schema_version: u32,
    pub expansion_policy_id: String,
    pub candidate_schema_id: String,
    pub dedupe_key: Vec<String>,
    pub dedupe_scope: DedupeScope,
    pub dry_rounds_required: u32,
    pub max_iterations: u32,
    pub max_spawned_nodes: u32,
    pub max_cost: f64,
    pub cost_currency: String,
    pub max_latency_ms: u64,
    pub candidate_disposition: CandidateDispositionContract,
    pub provenance: Provenance,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDispositionContract {
    UnreviewedMorphismProposal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExpansionFinding {
    pub code: String,
    pub detail: String,
}

pub fn validate_expansion_policy(policy: &ExpansionPolicy) -> Vec<ExpansionFinding> {
    let mut findings = Vec::new();
    if policy.schema != EXPANSION_POLICY_SCHEMA || policy.schema_version != 0 {
        push(
            &mut findings,
            "unsupported_policy_schema",
            "schema/version must name expansion.policy.v0",
        );
    }
    for (name, value) in [
        ("expansion_policy_id", &policy.expansion_policy_id),
        ("candidate_schema_id", &policy.candidate_schema_id),
        ("cost_currency", &policy.cost_currency),
        ("provenance.source", &policy.provenance.source),
        ("provenance.created_by", &policy.provenance.created_by),
    ] {
        if value.trim().is_empty() {
            push(
                &mut findings,
                "empty_required_field",
                format!("{name} must not be empty"),
            );
        }
    }
    if policy.dedupe_key.is_empty() || policy.dedupe_key.iter().any(|key| key.trim().is_empty()) {
        push(
            &mut findings,
            "missing_dedupe_key",
            "dedupe_key must contain non-empty fields",
        );
    }
    if policy.dedupe_key.iter().collect::<BTreeSet<_>>().len() != policy.dedupe_key.len() {
        push(
            &mut findings,
            "duplicate_dedupe_key",
            "dedupe_key fields must be unique",
        );
    }
    for (code, value) in [
        ("missing_dry_round_limit", policy.dry_rounds_required),
        ("missing_iteration_limit", policy.max_iterations),
        ("missing_node_limit", policy.max_spawned_nodes),
    ] {
        if value == 0 {
            push(&mut findings, code, "hard limit must be greater than zero");
        }
    }
    if !policy.max_cost.is_finite() || policy.max_cost <= 0.0 {
        push(
            &mut findings,
            "missing_cost_limit",
            "max_cost must be finite and greater than zero",
        );
    }
    if policy.max_latency_ms == 0 {
        push(
            &mut findings,
            "missing_latency_limit",
            "max_latency_ms must be greater than zero",
        );
    }
    findings.sort_by(|a, b| (&a.code, &a.detail).cmp(&(&b.code, &b.detail)));
    findings
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDispositionRequest {
    AcceptForProposal,
    Reject,
    Defer,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionCandidate {
    pub candidate_schema_id: String,
    pub dedupe_values: BTreeMap<String, String>,
    pub requested_disposition: CandidateDispositionRequest,
    pub topology_patch: TopologyPatch,
}

/// A strict replacement-style patch against one execution topology.
///
/// Collections are canonicalized by identity before hashing or application.
/// Updates carry the complete replacement value so applying a patch never
/// depends on merge-patch implementation details.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyPatch {
    pub schema: String,
    pub schema_version: u32,
    pub added_nodes: Vec<TopologyNode>,
    pub removed_node_ids: Vec<String>,
    pub updated_nodes: Vec<TopologyNode>,
    pub added_edges: Vec<TopologyEdge>,
    pub removed_edge_ids: Vec<String>,
}

/// The actual semantic delta computed from the before and after topologies.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyPatchDiff {
    pub added_node_ids: Vec<String>,
    pub removed_node_ids: Vec<String>,
    pub updated_node_ids: Vec<String>,
    pub added_edge_ids: Vec<String>,
    pub removed_edge_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDisposition {
    AcceptedForProposal,
    Rejected,
    Duplicate,
    Deferred,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpansionHalt {
    Continue,
    Dry,
    MaxIterations,
    MaxSpawnedNodes,
    MaxCost,
    MaxLatency,
    NeedsReview,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CandidateDecision {
    pub dedupe_fingerprint: String,
    pub disposition: CandidateDisposition,
    pub finding: Option<ExpansionFinding>,
    pub proposal_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExpansionProposal {
    pub proposal_id: String,
    pub base_topology_content_hash: String,
    pub proposed_topology_content_hash: String,
    pub review_status: &'static str,
    pub topology_patch: TopologyPatch,
    pub topology_diff: TopologyPatchDiff,
    pub morphism_proposal: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExpansionRoundResult {
    pub iteration: u32,
    pub dry_rounds: u32,
    pub total_cost: f64,
    pub total_latency_ms: u64,
    pub spawned_nodes: u32,
    pub halt: ExpansionHalt,
    pub findings: Vec<ExpansionFinding>,
    pub decisions: Vec<CandidateDecision>,
    pub proposals: Vec<ExpansionProposal>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReviewedTopologyTransition {
    pub proposal_id: String,
    pub accepted_review_id: String,
    pub accepted_revision_id: String,
    pub base_topology_content_hash: String,
    pub reviewed_topology_content_hash: String,
    pub accepted_graph_mutated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedExpansionReviewBinding {
    proposal_id: String,
    reviewed_topology_content_hash: String,
    review_id: String,
    revision_id: String,
}

/// Validates and canonicalizes a topology patch against its exact base.
pub fn canonical_topology_patch(
    base: &ExecutionTopology,
    patch: &TopologyPatch,
) -> Result<TopologyPatch, Vec<ExpansionFinding>> {
    let mut findings = Vec::new();
    if patch.schema != TOPOLOGY_PATCH_SCHEMA
        || patch.schema_version != TOPOLOGY_PATCH_SCHEMA_VERSION
    {
        push(
            &mut findings,
            "unsupported_topology_patch_schema",
            "schema/version must name topology.patch.v0",
        );
    }

    let base_nodes = base
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let base_edges = base
        .edges
        .iter()
        .map(|edge| (edge.edge_id.as_str(), edge))
        .collect::<BTreeMap<_, _>>();

    validate_entity_operations(
        patch.added_nodes.iter().map(|node| node.node_id.as_str()),
        patch.updated_nodes.iter().map(|node| node.node_id.as_str()),
        &patch.removed_node_ids,
        &base_nodes.keys().copied().collect(),
        "node",
        &mut findings,
    );
    validate_entity_operations(
        patch.added_edges.iter().map(|edge| edge.edge_id.as_str()),
        std::iter::empty(),
        &patch.removed_edge_ids,
        &base_edges.keys().copied().collect(),
        "edge",
        &mut findings,
    );
    for node in &patch.updated_nodes {
        if base_nodes.get(node.node_id.as_str()).copied() == Some(node) {
            push(
                &mut findings,
                "no_op_node_update",
                format!("updated node {} is unchanged", node.node_id),
            );
        }
    }
    if patch.added_nodes.is_empty()
        && patch.removed_node_ids.is_empty()
        && patch.updated_nodes.is_empty()
        && patch.added_edges.is_empty()
        && patch.removed_edge_ids.is_empty()
    {
        push(
            &mut findings,
            "empty_topology_patch",
            "topology patch must contain at least one operation",
        );
    }

    if !findings.is_empty() {
        findings.sort_by(|a, b| (&a.code, &a.detail).cmp(&(&b.code, &b.detail)));
        return Err(findings);
    }

    let mut canonical = patch.clone();
    canonical
        .added_nodes
        .sort_by(|a, b| a.node_id.cmp(&b.node_id));
    canonical.removed_node_ids.sort();
    canonical
        .updated_nodes
        .sort_by(|a, b| a.node_id.cmp(&b.node_id));
    canonical
        .added_edges
        .sort_by(|a, b| a.edge_id.cmp(&b.edge_id));
    canonical.removed_edge_ids.sort();
    Ok(canonical)
}

/// Applies a canonical typed patch and returns the actual before/after delta.
pub fn apply_topology_patch(
    base: &ExecutionTopology,
    patch: &TopologyPatch,
) -> Result<(ExecutionTopology, TopologyPatchDiff), Vec<ExpansionFinding>> {
    let patch = canonical_topology_patch(base, patch)?;
    apply_canonical_topology_patch(base, patch)
}

fn apply_canonical_topology_patch(
    base: &ExecutionTopology,
    patch: TopologyPatch,
) -> Result<(ExecutionTopology, TopologyPatchDiff), Vec<ExpansionFinding>> {
    let mut nodes = base
        .nodes
        .iter()
        .cloned()
        .map(|node| (node.node_id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let mut edges = base
        .edges
        .iter()
        .cloned()
        .map(|edge| (edge.edge_id.clone(), edge))
        .collect::<BTreeMap<_, _>>();

    for id in &patch.removed_edge_ids {
        edges.remove(id);
    }
    for id in &patch.removed_node_ids {
        nodes.remove(id);
    }
    for node in patch.updated_nodes {
        nodes.insert(node.node_id.clone(), node);
    }
    for node in patch.added_nodes {
        nodes.insert(node.node_id.clone(), node);
    }
    for edge in patch.added_edges {
        edges.insert(edge.edge_id.clone(), edge);
    }

    let mut after = base.clone();
    after.nodes = nodes.into_values().collect();
    after.edges = edges.into_values().collect();
    let topology_findings = validate_execution_topology(&after);
    if !topology_findings.is_empty() {
        return Err(topology_findings
            .into_iter()
            .map(|finding| {
                one(
                    "invalid_patched_topology",
                    format!(
                        "{} at {}: {}",
                        finding.code, finding.location, finding.detail
                    ),
                )
            })
            .collect());
    }
    Ok((after.clone(), topology_diff(base, &after)))
}

/// Computes the identity-level semantic difference between two topologies.
pub fn topology_diff(before: &ExecutionTopology, after: &ExecutionTopology) -> TopologyPatchDiff {
    let before_nodes = before
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let after_nodes = after
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let before_edges = before
        .edges
        .iter()
        .map(|edge| (edge.edge_id.as_str(), edge))
        .collect::<BTreeMap<_, _>>();
    let after_edges = after
        .edges
        .iter()
        .map(|edge| (edge.edge_id.as_str(), edge))
        .collect::<BTreeMap<_, _>>();
    TopologyPatchDiff {
        added_node_ids: after_nodes
            .keys()
            .filter(|id| !before_nodes.contains_key(**id))
            .map(|id| (*id).to_owned())
            .collect(),
        removed_node_ids: before_nodes
            .keys()
            .filter(|id| !after_nodes.contains_key(**id))
            .map(|id| (*id).to_owned())
            .collect(),
        updated_node_ids: after_nodes
            .iter()
            .filter(|(id, node)| before_nodes.get(**id).is_some_and(|before| before != *node))
            .map(|(id, _)| (*id).to_owned())
            .collect(),
        added_edge_ids: after_edges
            .keys()
            .filter(|id| !before_edges.contains_key(**id))
            .map(|id| (*id).to_owned())
            .collect(),
        removed_edge_ids: before_edges
            .keys()
            .filter(|id| !after_edges.contains_key(**id))
            .map(|id| (*id).to_owned())
            .collect(),
    }
}

fn validate_entity_operations<'a>(
    added: impl Iterator<Item = &'a str>,
    updated: impl Iterator<Item = &'a str>,
    removed: &'a [String],
    existing: &BTreeSet<&'a str>,
    kind: &str,
    findings: &mut Vec<ExpansionFinding>,
) {
    let added = added.collect::<Vec<_>>();
    let updated = updated.collect::<Vec<_>>();
    let removed = removed.iter().map(String::as_str).collect::<Vec<_>>();
    for (operation, ids) in [
        ("added", &added),
        ("updated", &updated),
        ("removed", &removed),
    ] {
        if ids.iter().any(|id| id.trim().is_empty()) {
            push(
                findings,
                "empty_patch_id",
                format!("{operation} {kind} id must not be empty"),
            );
        }
        if ids.iter().copied().collect::<BTreeSet<_>>().len() != ids.len() {
            push(
                findings,
                "duplicate_patch_id",
                format!("{operation} {kind} ids must be unique"),
            );
        }
    }
    for id in &added {
        if existing.contains(id) {
            push(
                findings,
                "patch_addition_exists",
                format!("{kind} {id} already exists"),
            );
        }
    }
    for id in updated.iter().chain(&removed) {
        if !existing.contains(id) {
            push(
                findings,
                "patch_target_missing",
                format!("{kind} {id} does not exist"),
            );
        }
    }
    let operation_count = added
        .iter()
        .chain(&updated)
        .chain(&removed)
        .copied()
        .collect::<Vec<_>>();
    if operation_count
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .len()
        != operation_count.len()
    {
        push(
            findings,
            "conflicting_patch_operations",
            format!("{kind} id occurs in multiple operations"),
        );
    }
}

/// Binds an expansion proposal to the opaque reviewed-topology authority that
/// [`crate::graph_compiler::reviewed_compilation_mode`] derived from the
/// canonical CaseGraphen review log. Proposal mode and hash substitution fail.
pub fn accepted_expansion_review_binding(
    compilation_mode: &CompilationMode,
    proposal_id: &str,
    reviewed_topology: &ExecutionTopology,
) -> Result<AcceptedExpansionReviewBinding, ExpansionFinding> {
    let binding = compilation_mode.reviewed_binding().ok_or_else(|| {
        one(
            "accepted_review_required",
            "expansion acceptance requires a canonical reviewed topology binding",
        )
    })?;
    let reviewed_hash =
        execution_topology_content_hash(reviewed_topology).expect("typed topology serializes");
    if proposal_id.trim().is_empty()
        || binding.topology_content_hash() != reviewed_hash
        || binding.expansion_proposal_id() != Some(proposal_id)
    {
        return Err(one(
            "accepted_review_binding_mismatch",
            "reviewed binding must name the exact proposed topology hash and proposal id",
        ));
    }
    Ok(AcceptedExpansionReviewBinding {
        proposal_id: proposal_id.to_owned(),
        reviewed_topology_content_hash: reviewed_hash,
        review_id: binding.review_id().to_owned(),
        revision_id: binding.base_revision_id().to_owned(),
    })
}

pub struct ExpansionController {
    policy: ExpansionPolicy,
    base_topology: ExecutionTopology,
    base_topology_hash: String,
    topology_id: String,
    case_space_id: String,
    active_attempt: Option<String>,
    seen: BTreeSet<String>,
    iteration: u32,
    dry_rounds: u32,
    spawned_nodes: u32,
    total_cost: f64,
    total_latency_ms: u64,
    proposals: BTreeMap<String, ExpansionProposal>,
}

impl ExpansionController {
    pub fn new(
        policy: ExpansionPolicy,
        topology: &ExecutionTopology,
    ) -> Result<Self, Vec<ExpansionFinding>> {
        let findings = validate_expansion_policy(&policy);
        if !findings.is_empty() {
            return Err(findings);
        }
        Ok(Self {
            policy,
            base_topology: topology.clone(),
            base_topology_hash: execution_topology_content_hash(topology)
                .expect("typed topology serializes"),
            topology_id: topology.topology_id.clone(),
            case_space_id: topology.case_space_id.clone(),
            active_attempt: None,
            seen: BTreeSet::new(),
            iteration: 0,
            dry_rounds: 0,
            spawned_nodes: 0,
            total_cost: 0.0,
            total_latency_ms: 0,
            proposals: BTreeMap::new(),
        })
    }

    pub fn begin_attempt(
        &mut self,
        attempt_id: &str,
        topology: &ExecutionTopology,
    ) -> Result<(), ExpansionFinding> {
        let hash = execution_topology_content_hash(topology).expect("typed topology serializes");
        if hash != self.base_topology_hash {
            return Err(one(
                "topology_hash_switch",
                "an expansion attempt cannot switch topology content hash",
            ));
        }
        match self.active_attempt.as_deref() {
            Some(active) if active != attempt_id => Err(one(
                "attempt_in_progress",
                format!("attempt {active} is still active"),
            )),
            _ if attempt_id.trim().is_empty() => {
                Err(one("empty_attempt_id", "attempt_id must not be empty"))
            }
            _ => {
                self.active_attempt = Some(attempt_id.to_owned());
                Ok(())
            }
        }
    }

    pub fn finish_attempt(&mut self, attempt_id: &str) -> Result<(), ExpansionFinding> {
        if self.active_attempt.as_deref() != Some(attempt_id) {
            return Err(one(
                "attempt_mismatch",
                "only the active attempt may finish",
            ));
        }
        self.active_attempt = None;
        Ok(())
    }

    pub fn process_round(
        &mut self,
        attempt_id: &str,
        candidates: Vec<ExpansionCandidate>,
        accounted_round_cost: f64,
        accounted_round_latency_ms: u64,
    ) -> Result<ExpansionRoundResult, ExpansionFinding> {
        if self.active_attempt.as_deref() != Some(attempt_id) {
            return Err(one(
                "attempt_mismatch",
                "round must belong to the active attempt",
            ));
        }
        if self.iteration >= self.policy.max_iterations {
            return Ok(self.result(
                ExpansionHalt::MaxIterations,
                vec![one(
                    "max_iterations_reached",
                    "max_iterations prevents another round",
                )],
                vec![],
                vec![],
            ));
        }
        if !accounted_round_cost.is_finite() || accounted_round_cost < 0.0 {
            return Err(one(
                "invalid_accounted_round_cost",
                "accounted_round_cost must be finite and non-negative",
            ));
        }
        self.iteration += 1;
        self.total_cost += accounted_round_cost;
        self.total_latency_ms = self
            .total_latency_ms
            .saturating_add(accounted_round_latency_ms);
        let cost_exhausted = self.total_cost >= self.policy.max_cost;
        let latency_exhausted = self.total_latency_ms >= self.policy.max_latency_ms;
        let mut unseen = 0_u32;
        let mut decisions = Vec::new();
        let mut created = Vec::new();
        for candidate in candidates {
            let fingerprint = match fingerprint(&self.policy, &candidate) {
                Ok(value) => value,
                Err(finding) => {
                    decisions.push(CandidateDecision {
                        dedupe_fingerprint: String::new(),
                        disposition: CandidateDisposition::Rejected,
                        finding: Some(finding),
                        proposal_id: None,
                    });
                    continue;
                }
            };
            if !self.seen.insert(fingerprint.clone()) {
                decisions.push(CandidateDecision {
                    dedupe_fingerprint: fingerprint,
                    disposition: CandidateDisposition::Duplicate,
                    finding: None,
                    proposal_id: None,
                });
                continue;
            }
            unseen += 1;
            if cost_exhausted || latency_exhausted {
                decisions.push(CandidateDecision {
                    dedupe_fingerprint: fingerprint,
                    disposition: CandidateDisposition::Deferred,
                    finding: Some(if cost_exhausted {
                        one("max_cost_reached", "round reached max_cost")
                    } else {
                        one("max_latency_reached", "round reached max_latency_ms")
                    }),
                    proposal_id: None,
                });
                continue;
            }
            let (disposition, proposal, finding) = match candidate.requested_disposition {
                CandidateDispositionRequest::Reject => (CandidateDisposition::Rejected, None, None),
                CandidateDispositionRequest::Defer => (CandidateDisposition::Deferred, None, None),
                CandidateDispositionRequest::AcceptForProposal => {
                    match proposal(&self.base_topology, candidate.topology_patch) {
                        Err(finding) => (CandidateDisposition::Rejected, None, Some(finding)),
                        Ok(proposal) => {
                            let added_node_count =
                                u32::try_from(proposal.topology_diff.added_node_ids.len())
                                    .unwrap_or(u32::MAX);
                            let remaining = self
                                .policy
                                .max_spawned_nodes
                                .saturating_sub(self.spawned_nodes);
                            if added_node_count > remaining {
                                (
                                    CandidateDisposition::Deferred,
                                    None,
                                    Some(one(
                                        "max_spawned_nodes_reached",
                                        format!(
                                            "patch adds {added_node_count} nodes but only {remaining} remain",
                                        ),
                                    )),
                                )
                            } else if self.proposals.contains_key(&proposal.proposal_id) {
                                (
                                    CandidateDisposition::Duplicate,
                                    None,
                                    Some(one(
                                        "duplicate_proposal",
                                        "canonical patch was already proposed against this base",
                                    )),
                                )
                            } else {
                                self.spawned_nodes =
                                    self.spawned_nodes.saturating_add(added_node_count);
                                (
                                    CandidateDisposition::AcceptedForProposal,
                                    Some(proposal),
                                    None,
                                )
                            }
                        }
                    }
                }
            };
            let proposal_id = proposal
                .as_ref()
                .map(|proposal| proposal.proposal_id.clone());
            if let Some(proposal) = proposal {
                self.proposals
                    .insert(proposal.proposal_id.clone(), proposal.clone());
                created.push(proposal);
            }
            decisions.push(CandidateDecision {
                dedupe_fingerprint: fingerprint,
                disposition,
                finding,
                proposal_id,
            });
        }
        self.dry_rounds = if unseen == 0 { self.dry_rounds + 1 } else { 0 };
        let halt = if cost_exhausted {
            ExpansionHalt::MaxCost
        } else if latency_exhausted {
            ExpansionHalt::MaxLatency
        } else if self.spawned_nodes >= self.policy.max_spawned_nodes {
            ExpansionHalt::MaxSpawnedNodes
        } else if self.iteration >= self.policy.max_iterations {
            ExpansionHalt::MaxIterations
        } else if self.dry_rounds >= self.policy.dry_rounds_required {
            ExpansionHalt::Dry
        } else if !created.is_empty() {
            ExpansionHalt::NeedsReview
        } else {
            ExpansionHalt::Continue
        };
        let findings = match halt {
            ExpansionHalt::MaxCost => vec![one(
                "max_cost_reached",
                "max_cost prevents further expansion",
            )],
            ExpansionHalt::MaxLatency => vec![one(
                "max_latency_reached",
                "max_latency_ms prevents further expansion",
            )],
            ExpansionHalt::MaxSpawnedNodes => vec![one(
                "max_spawned_nodes_reached",
                "max_spawned_nodes prevents further expansion",
            )],
            ExpansionHalt::MaxIterations => vec![one(
                "max_iterations_reached",
                "max_iterations prevents further expansion",
            )],
            ExpansionHalt::Dry => vec![one(
                "dry_round_limit_reached",
                "consecutive dry rounds reached the termination limit",
            )],
            ExpansionHalt::Continue | ExpansionHalt::NeedsReview => vec![],
        };
        Ok(self.result(halt, findings, decisions, created))
    }

    pub fn review_accepted(
        &self,
        proposal_id: &str,
        reviewed_topology: &ExecutionTopology,
        accepted_review: &AcceptedExpansionReviewBinding,
    ) -> Result<ReviewedTopologyTransition, ExpansionFinding> {
        let reviewed_hash =
            execution_topology_content_hash(reviewed_topology).expect("typed topology serializes");
        let proposal = self.proposals.get(proposal_id).ok_or_else(|| {
            one(
                "unknown_proposal",
                "proposal_id was not emitted by this controller",
            )
        })?;
        if accepted_review.proposal_id != proposal_id
            || accepted_review.reviewed_topology_content_hash != reviewed_hash
        {
            return Err(one(
                "invalid_accepted_review_binding",
                "accepted review binding must name this proposal and exact topology hash",
            ));
        }
        if reviewed_topology.topology_id != self.topology_id
            || reviewed_topology.case_space_id != self.case_space_id
        {
            return Err(one(
                "reviewed_topology_identity_mismatch",
                "reviewed topology must preserve topology_id and case_space_id",
            ));
        }
        if reviewed_hash != proposal.proposed_topology_content_hash {
            return Err(one(
                "reviewed_topology_patch_mismatch",
                "reviewed topology must equal the exact deterministic patch application",
            ));
        }
        if let Some(finding) = lint_execution_topology(reviewed_topology)
            .findings
            .into_iter()
            .find(|finding| {
                finding.classification == FindingClassification::Deterministic
                    && finding.severity == LintSeverity::Error
            })
        {
            return Err(one(
                "reviewed_topology_invalid",
                format!("{}: {}", finding.code, finding.detail),
            ));
        }
        if reviewed_hash == self.base_topology_hash {
            return Err(one(
                "unchanged_reviewed_topology",
                "accepted review must produce a distinct topology content hash",
            ));
        }
        Ok(ReviewedTopologyTransition {
            proposal_id: proposal_id.to_owned(),
            accepted_review_id: accepted_review.review_id.clone(),
            accepted_revision_id: accepted_review.revision_id.clone(),
            base_topology_content_hash: self.base_topology_hash.clone(),
            reviewed_topology_content_hash: reviewed_hash,
            accepted_graph_mutated: false,
        })
    }

    fn result(
        &self,
        halt: ExpansionHalt,
        findings: Vec<ExpansionFinding>,
        decisions: Vec<CandidateDecision>,
        proposals: Vec<ExpansionProposal>,
    ) -> ExpansionRoundResult {
        ExpansionRoundResult {
            iteration: self.iteration,
            dry_rounds: self.dry_rounds,
            total_cost: self.total_cost,
            total_latency_ms: self.total_latency_ms,
            spawned_nodes: self.spawned_nodes,
            halt,
            findings,
            decisions,
            proposals,
        }
    }
}

fn fingerprint(
    policy: &ExpansionPolicy,
    candidate: &ExpansionCandidate,
) -> Result<String, ExpansionFinding> {
    if candidate.candidate_schema_id != policy.candidate_schema_id {
        return Err(one(
            "candidate_schema_mismatch",
            "candidate schema does not match policy",
        ));
    }
    let mut selected = BTreeMap::new();
    for key in &policy.dedupe_key {
        let value = candidate
            .dedupe_values
            .get(key)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                one(
                    "missing_dedupe_value",
                    format!("candidate lacks dedupe field {key}"),
                )
            })?;
        selected.insert(key, value);
    }
    Ok(format!(
        "sha256:{}",
        hash(&serde_json::to_vec(&selected).expect("dedupe map serializes"))
    ))
}

fn proposal(
    base: &ExecutionTopology,
    patch: TopologyPatch,
) -> Result<ExpansionProposal, ExpansionFinding> {
    let base_hash = execution_topology_content_hash(base).expect("typed topology serializes");
    let canonical_patch = canonical_topology_patch(base, &patch).map_err(|findings| {
        findings
            .into_iter()
            .next()
            .expect("invalid patch has a finding")
    })?;
    let (proposed_topology, topology_diff) =
        apply_canonical_topology_patch(base, canonical_patch.clone()).map_err(|findings| {
            findings
                .into_iter()
                .next()
                .expect("invalid patch has a finding")
        })?;
    let proposed_hash =
        execution_topology_content_hash(&proposed_topology).expect("typed topology serializes");
    let material =
        serde_json::to_vec(&(&base_hash, &canonical_patch)).expect("proposal serializes");
    let id = format!("proposal:sha256-{}", hash(&material));
    Ok(ExpansionProposal {
        proposal_id: id.clone(),
        base_topology_content_hash: base_hash,
        proposed_topology_content_hash: proposed_hash,
        review_status: "unreviewed",
        topology_patch: canonical_patch,
        topology_diff,
        morphism_proposal: serde_json::json!({"proposal_id":id,"review_status":"unreviewed","accepted_graph_mutated":false}),
    })
}

fn hash(bytes: &[u8]) -> String {
    crate::native_hash::sha256_hex(bytes)
}
fn push(findings: &mut Vec<ExpansionFinding>, code: &str, detail: impl Into<String>) {
    findings.push(one(code, detail));
}
fn one(code: &str, detail: impl Into<String>) -> ExpansionFinding {
    ExpansionFinding {
        code: code.to_owned(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_topology::parse_execution_topology;

    #[test]
    fn accepted_review_cannot_substitute_a_different_topology() {
        let base = parse_execution_topology(include_str!(
            "../schemas/experimental/execution.topology.file-review.example.json"
        ))
        .unwrap();
        let policy: ExpansionPolicy = serde_json::from_str(include_str!(
            "../schemas/experimental/expansion.policy.example.json"
        ))
        .unwrap();
        let mut added = base.nodes[0].clone();
        added.node_id = "node:reviewed-addition".into();
        added.work_cell_id = "work:reviewed-addition".into();
        added.idempotency_key = "expand:reviewed-addition".into();
        let patch = TopologyPatch {
            schema: TOPOLOGY_PATCH_SCHEMA.into(),
            schema_version: TOPOLOGY_PATCH_SCHEMA_VERSION,
            added_nodes: vec![added],
            removed_node_ids: vec![],
            updated_nodes: vec![],
            added_edges: vec![],
            removed_edge_ids: vec![],
        };
        let mut controller = ExpansionController::new(policy, &base).unwrap();
        controller
            .begin_attempt("attempt:substitution", &base)
            .unwrap();
        let proposal = controller
            .process_round(
                "attempt:substitution",
                vec![ExpansionCandidate {
                    candidate_schema_id: "schema:bug-candidate".into(),
                    dedupe_values: BTreeMap::from([
                        ("file".into(), "substitution.rs".into()),
                        ("symbol".into(), "handler".into()),
                        ("failure_signature".into(), "panic-substitution".into()),
                    ]),
                    requested_disposition: CandidateDispositionRequest::AcceptForProposal,
                    topology_patch: patch.clone(),
                }],
                0.0,
                0,
            )
            .unwrap()
            .proposals
            .remove(0);

        let (mut substituted, _) = apply_topology_patch(&base, &patch).unwrap();
        substituted.nodes[0]
            .purpose
            .push_str(" substituted after review");
        let substituted_hash =
            execution_topology_content_hash(&substituted).expect("typed topology serializes");
        let forged_binding = AcceptedExpansionReviewBinding {
            proposal_id: proposal.proposal_id.clone(),
            reviewed_topology_content_hash: substituted_hash,
            review_id: "review:forged-substitution".into(),
            revision_id: "revision:observed".into(),
        };
        assert_eq!(
            controller
                .review_accepted(&proposal.proposal_id, &substituted, &forged_binding)
                .unwrap_err()
                .code,
            "reviewed_topology_patch_mismatch"
        );
    }
}

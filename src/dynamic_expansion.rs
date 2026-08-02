//! Bounded dynamic-topology discovery producing reviewable proposals only.

use crate::{
    execution_topology::{execution_topology_content_hash, ExecutionTopology, Provenance},
    graph_compiler::CompilationMode,
    graph_lint::{lint_execution_topology, FindingClassification, LintSeverity},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const EXPANSION_POLICY_SCHEMA: &str = "casegraphen.experimental.expansion.policy.v0";

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
    pub topology_patch: Value,
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
    pub topology_content_hash: String,
    pub review_status: &'static str,
    pub topology_patch: Value,
    pub morphism_proposal: Value,
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
            let disposition = match candidate.requested_disposition {
                CandidateDispositionRequest::Reject => CandidateDisposition::Rejected,
                CandidateDispositionRequest::Defer => CandidateDisposition::Deferred,
                CandidateDispositionRequest::AcceptForProposal
                    if self.spawned_nodes >= self.policy.max_spawned_nodes =>
                {
                    CandidateDisposition::Deferred
                }
                CandidateDispositionRequest::AcceptForProposal => {
                    CandidateDisposition::AcceptedForProposal
                }
            };
            let proposal = (disposition == CandidateDisposition::AcceptedForProposal).then(|| {
                self.spawned_nodes += 1;
                proposal(&self.base_topology_hash, candidate.topology_patch)
            });
            let proposal_id = proposal
                .as_ref()
                .map(|proposal| proposal.proposal_id.clone());
            if let Some(proposal) = proposal {
                self.proposals
                    .insert(proposal.proposal_id.clone(), proposal.clone());
                created.push(proposal);
            }
            let finding = (disposition == CandidateDisposition::Deferred
                && self.spawned_nodes >= self.policy.max_spawned_nodes)
                .then(|| {
                    one(
                        "max_spawned_nodes_reached",
                        "candidate would exceed max_spawned_nodes",
                    )
                });
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
        if !self.proposals.contains_key(proposal_id) {
            return Err(one(
                "unknown_proposal",
                "proposal_id was not emitted by this controller",
            ));
        }
        let reviewed_hash =
            execution_topology_content_hash(reviewed_topology).expect("typed topology serializes");
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

fn proposal(base_hash: &str, patch: Value) -> ExpansionProposal {
    let material = serde_json::to_vec(&(base_hash, &patch)).expect("proposal serializes");
    let id = format!("proposal:sha256-{}", hash(&material));
    ExpansionProposal {
        proposal_id: id.clone(),
        topology_content_hash: base_hash.to_owned(),
        review_status: "unreviewed",
        topology_patch: patch,
        morphism_proposal: serde_json::json!({"proposal_id":id,"review_status":"unreviewed","accepted_graph_mutated":false}),
    }
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

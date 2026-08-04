//! Pure audit-to-redesign proposal and disposition history.
//!
//! Accepted bindings record an ordinary review decision. This module exposes
//! no API that mutates an accepted topology or case ledger.

use crate::{
    execution_topology::{execution_topology_content_hash, ExecutionTopology},
    graph_lint::lint_execution_topology,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const REDESIGN_PROPOSAL_SCHEMA: &str = "casegraphen.experimental.topology.redesign_proposal.v0";
pub const REDESIGN_DISPOSITION_LOG_SCHEMA: &str =
    "casegraphen.experimental.topology.redesign_disposition_log.v0";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EntityChange {
    pub id: String,
    pub old_content_hash: String,
    pub new_content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetChange {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PolicySetChanges {
    pub verification: SetChange,
    pub budget: SetChange,
    pub expansion: SetChange,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TopologyVersionDiff {
    pub old_topology_content_hash: String,
    pub proposed_topology_content_hash: String,
    pub added_node_ids: Vec<String>,
    pub removed_node_ids: Vec<String>,
    pub changed_nodes: Vec<EntityChange>,
    pub added_edge_ids: Vec<String>,
    pub removed_edge_ids: Vec<String>,
    pub changed_edges: Vec<EntityChange>,
    pub policy_changes: PolicySetChanges,
}

pub fn diff_topology_versions(
    old: &ExecutionTopology,
    proposed: &ExecutionTopology,
) -> TopologyVersionDiff {
    let (added_node_ids, removed_node_ids, changed_nodes) = diff_entities(
        old.nodes.iter().map(|node| (node.node_id.as_str(), node)),
        proposed
            .nodes
            .iter()
            .map(|node| (node.node_id.as_str(), node)),
        canonical_node_hash,
    );
    let (added_edge_ids, removed_edge_ids, changed_edges) = diff_entities(
        old.edges.iter().map(|edge| (edge.edge_id.as_str(), edge)),
        proposed
            .edges
            .iter()
            .map(|edge| (edge.edge_id.as_str(), edge)),
        canonical_edge_hash,
    );
    TopologyVersionDiff {
        old_topology_content_hash: execution_topology_content_hash(old)
            .expect("typed topology serializes"),
        proposed_topology_content_hash: execution_topology_content_hash(proposed)
            .expect("typed topology serializes"),
        added_node_ids,
        removed_node_ids,
        changed_nodes,
        added_edge_ids,
        removed_edge_ids,
        changed_edges,
        policy_changes: PolicySetChanges {
            verification: set_change(
                &old.verification_policy_ids,
                &proposed.verification_policy_ids,
            ),
            budget: set_change(&old.budget_policy_ids, &proposed.budget_policy_ids),
            expansion: set_change(&old.expansion_policy_ids, &proposed.expansion_policy_ids),
        },
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedesignEvidenceRefs {
    pub audit_artifact_ids: Vec<String>,
    pub integration_proposal_ids: Vec<String>,
    pub expansion_proposal_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedImpact {
    pub metric: String,
    pub expected_direction: String,
    pub estimated_delta: Option<f64>,
    pub rationale: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewerAuthorityRequirement {
    pub authority_policy_id: String,
    pub required_capability_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationRefs {
    pub input_artifact_id: String,
    pub old_report_artifact_id: String,
    pub proposed_report_artifact_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RedesignProposal {
    pub schema: &'static str,
    pub schema_version: u32,
    pub proposal_id: String,
    pub review_status: &'static str,
    pub base_topology_content_hash: String,
    pub proposed_topology_content_hash: String,
    pub evidence: RedesignEvidenceRefs,
    pub changes: TopologyVersionDiff,
    pub expected_impact: Vec<ExpectedImpact>,
    pub uncertainty: Vec<String>,
    pub information_loss: Vec<String>,
    pub reviewer_authority: ReviewerAuthorityRequirement,
    pub simulation: SimulationRefs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RedesignFinding {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedesignProposalInput {
    pub evidence: RedesignEvidenceRefs,
    pub expected_impact: Vec<ExpectedImpact>,
    pub uncertainty: Vec<String>,
    pub information_loss: Vec<String>,
    pub reviewer_authority: ReviewerAuthorityRequirement,
    pub simulation: SimulationRefs,
}

pub fn propose_redesign(
    old: &ExecutionTopology,
    proposed: &ExecutionTopology,
    mut input: RedesignProposalInput,
) -> Result<RedesignProposal, Vec<RedesignFinding>> {
    normalize_proposal_input(&mut input);
    let mut findings = Vec::new();
    if old.topology_id != proposed.topology_id || old.case_space_id != proposed.case_space_id {
        findings.push(one(
            "topology_identity_mismatch",
            "redesign must preserve topology_id and case_space_id",
        ));
    }
    if input.evidence.audit_artifact_ids.is_empty() {
        findings.push(one(
            "missing_audit_artifact",
            "at least one audit artifact id is required",
        ));
    }
    for id in input
        .evidence
        .audit_artifact_ids
        .iter()
        .chain(input.simulation_ids())
    {
        if !is_artifact_id(id) {
            findings.push(one(
                "invalid_artifact_id",
                format!("{id} must be content addressed"),
            ));
        }
    }
    for id in &input.evidence.integration_proposal_ids {
        if !is_prefixed_hash_id(id, "proposal:sha256-") {
            findings.push(one(
                "invalid_integration_proposal_id",
                format!("{id} must be a #48 content-addressed proposal id"),
            ));
        }
    }
    for id in &input.evidence.expansion_proposal_ids {
        if !is_prefixed_hash_id(id, "proposal:sha256-") {
            findings.push(one(
                "invalid_expansion_proposal_id",
                format!("{id} must be a #54 content-addressed proposal id"),
            ));
        }
    }
    if input.expected_impact.is_empty() {
        findings.push(one(
            "missing_expected_impact",
            "expected impact must be explicit",
        ));
    }
    for impact in &input.expected_impact {
        if impact.metric.trim().is_empty()
            || impact.expected_direction.trim().is_empty()
            || impact.rationale.trim().is_empty()
            || impact
                .estimated_delta
                .is_some_and(|delta| !delta.is_finite())
        {
            findings.push(one(
                "invalid_expected_impact",
                "impact metric, direction, rationale, and optional finite delta are required",
            ));
        }
    }
    if input.uncertainty.is_empty() {
        findings.push(one("missing_uncertainty", "uncertainty must be explicit"));
    }
    if input.information_loss.is_empty() {
        findings.push(one(
            "missing_information_loss",
            "information loss must be explicit, including none",
        ));
    }
    if input
        .uncertainty
        .iter()
        .any(|value| value.trim().is_empty())
        || input
            .information_loss
            .iter()
            .any(|value| value.trim().is_empty())
    {
        findings.push(one(
            "empty_risk_statement",
            "uncertainty and information-loss statements must be non-empty",
        ));
    }
    if input.reviewer_authority.authority_policy_id.is_empty()
        || input.reviewer_authority.required_capability_ids.is_empty()
        || input
            .reviewer_authority
            .required_capability_ids
            .iter()
            .any(|id| id.trim().is_empty())
    {
        findings.push(one(
            "missing_reviewer_authority",
            "reviewer authority policy and capabilities are required",
        ));
    }
    for finding in lint_execution_topology(proposed)
        .findings
        .into_iter()
        .filter(|finding| finding.is_deterministic_error())
    {
        findings.push(one(
            "invalid_proposed_topology",
            format!("{}: {}", finding.code, finding.detail),
        ));
    }
    let changes = diff_topology_versions(old, proposed);
    if changes.old_topology_content_hash == changes.proposed_topology_content_hash {
        findings.push(one(
            "empty_redesign",
            "proposal must change the canonical topology hash",
        ));
    }
    findings.sort_by(|a, b| (&a.code, &a.detail).cmp(&(&b.code, &b.detail)));
    if !findings.is_empty() {
        return Err(findings);
    }
    let material = serde_json::to_vec(&(
        &changes,
        &input.evidence,
        &input.expected_impact,
        &input.uncertainty,
        &input.information_loss,
        &input.reviewer_authority,
        &input.simulation,
    ))
    .expect("proposal material serializes");
    Ok(RedesignProposal {
        schema: REDESIGN_PROPOSAL_SCHEMA,
        schema_version: 0,
        proposal_id: format!(
            "redesign:sha256-{}",
            crate::native_hash::sha256_hex(&material)
        ),
        review_status: "unreviewed",
        base_topology_content_hash: changes.old_topology_content_hash.clone(),
        proposed_topology_content_hash: changes.proposed_topology_content_hash.clone(),
        evidence: input.evidence,
        changes,
        expected_impact: input.expected_impact,
        uncertainty: input.uncertainty,
        information_loss: input.information_loss,
        reviewer_authority: input.reviewer_authority,
        simulation: input.simulation,
    })
}

impl RedesignProposalInput {
    fn simulation_ids(&self) -> impl Iterator<Item = &String> {
        [
            &self.simulation.input_artifact_id,
            &self.simulation.old_report_artifact_id,
            &self.simulation.proposed_report_artifact_id,
        ]
        .into_iter()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RedesignDisposition {
    Proposed,
    Rejected {
        review_id: String,
        revision_id: String,
        reason: String,
    },
    Superseded {
        review_id: String,
        revision_id: String,
        superseding_proposal_id: String,
        reason: String,
    },
    AcceptedBinding {
        review_id: String,
        revision_id: String,
        reviewer_authority_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RedesignDispositionEntry {
    pub sequence: u64,
    pub proposal_id: String,
    pub previous_entry_hash: Option<String>,
    pub entry_hash: String,
    pub disposition: RedesignDisposition,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RedesignDispositionLog {
    pub schema: &'static str,
    pub schema_version: u32,
    pub entries: Vec<RedesignDispositionEntry>,
}

impl RedesignDispositionLog {
    pub fn new(proposal: &RedesignProposal) -> Self {
        let entry = disposition_entry(
            0,
            &proposal.proposal_id,
            None,
            RedesignDisposition::Proposed,
        );
        Self {
            schema: REDESIGN_DISPOSITION_LOG_SCHEMA,
            schema_version: 0,
            entries: vec![entry],
        }
    }

    pub fn append(
        &mut self,
        proposal: &RedesignProposal,
        disposition: RedesignDisposition,
    ) -> Result<&RedesignDispositionEntry, RedesignFinding> {
        if self
            .entries
            .first()
            .map_or(true, |entry| entry.proposal_id != proposal.proposal_id)
        {
            return Err(one(
                "proposal_log_mismatch",
                "log is bound to another proposal",
            ));
        }
        if self.entries.len() != 1 {
            return Err(one(
                "terminal_disposition_exists",
                "proposal already has a terminal disposition",
            ));
        }
        validate_disposition(&disposition)?;
        if let RedesignDisposition::AcceptedBinding {
            reviewer_authority_id,
            ..
        } = &disposition
        {
            if reviewer_authority_id != &proposal.reviewer_authority.authority_policy_id {
                return Err(one(
                    "reviewer_authority_mismatch",
                    "accepted binding must name the proposal's required authority policy",
                ));
            }
        }
        let previous = self.entries.last().map(|entry| entry.entry_hash.clone());
        let entry = disposition_entry(
            self.entries.len() as u64,
            &proposal.proposal_id,
            previous,
            disposition,
        );
        self.entries.push(entry);
        Ok(self.entries.last().expect("entry appended"))
    }
}

fn validate_disposition(disposition: &RedesignDisposition) -> Result<(), RedesignFinding> {
    match disposition {
        RedesignDisposition::Proposed => Err(one(
            "duplicate_proposed",
            "proposed is created only by log genesis",
        )),
        RedesignDisposition::Rejected {
            review_id,
            revision_id,
            reason,
        } if review_id.is_empty() || revision_id.is_empty() || reason.is_empty() => Err(one(
            "invalid_rejection",
            "review, revision, and reason are required",
        )),
        RedesignDisposition::Superseded {
            review_id,
            revision_id,
            superseding_proposal_id,
            reason,
        } if review_id.is_empty()
            || revision_id.is_empty()
            || !is_prefixed_hash_id(superseding_proposal_id, "redesign:sha256-")
            || reason.is_empty() =>
        {
            Err(one(
                "invalid_supersession",
                "review, revision, successor, and reason are required",
            ))
        }
        RedesignDisposition::AcceptedBinding {
            review_id,
            revision_id,
            reviewer_authority_id,
        } if review_id.is_empty() || revision_id.is_empty() || reviewer_authority_id.is_empty() => {
            Err(one(
                "invalid_accepted_binding",
                "normal review id/revision and reviewer authority are required",
            ))
        }
        _ => Ok(()),
    }
}

fn disposition_entry(
    sequence: u64,
    proposal_id: &str,
    previous: Option<String>,
    disposition: RedesignDisposition,
) -> RedesignDispositionEntry {
    let material = serde_json::to_vec(&(sequence, proposal_id, &previous, &disposition))
        .expect("entry serializes");
    RedesignDispositionEntry {
        sequence,
        proposal_id: proposal_id.to_owned(),
        previous_entry_hash: previous,
        entry_hash: crate::native_hash::sha256_hex(&material),
        disposition,
    }
}

fn diff_entities<'a, T: 'a>(
    old: impl Iterator<Item = (&'a str, &'a T)>,
    proposed: impl Iterator<Item = (&'a str, &'a T)>,
    hash_entity: fn(&T) -> String,
) -> (Vec<String>, Vec<String>, Vec<EntityChange>) {
    let old = old
        .map(|(id, value)| (id, hash_entity(value)))
        .collect::<BTreeMap<_, _>>();
    let proposed = proposed
        .map(|(id, value)| (id, hash_entity(value)))
        .collect::<BTreeMap<_, _>>();
    let added = proposed
        .keys()
        .filter(|id| !old.contains_key(**id))
        .map(|id| (*id).to_owned())
        .collect();
    let removed = old
        .keys()
        .filter(|id| !proposed.contains_key(**id))
        .map(|id| (*id).to_owned())
        .collect();
    let changed = old
        .iter()
        .filter_map(|(id, old_hash)| {
            proposed
                .get(id)
                .filter(|new_hash| *new_hash != old_hash)
                .map(|new_hash| EntityChange {
                    id: (*id).to_owned(),
                    old_content_hash: old_hash.clone(),
                    new_content_hash: new_hash.clone(),
                })
        })
        .collect();
    (added, removed, changed)
}

fn set_change(old: &[String], proposed: &[String]) -> SetChange {
    let old = old.iter().collect::<BTreeSet<_>>();
    let proposed = proposed.iter().collect::<BTreeSet<_>>();
    SetChange {
        added: proposed.difference(&old).map(|id| (*id).clone()).collect(),
        removed: old.difference(&proposed).map(|id| (*id).clone()).collect(),
    }
}
fn entity_hash(value: &impl Serialize) -> String {
    crate::native_hash::sha256_hex(&serde_json::to_vec(value).expect("entity serializes"))
}
fn canonical_node_hash(node: &crate::execution_topology::TopologyNode) -> String {
    let mut node = node.clone();
    node.inputs.sort_by(|a, b| a.name.cmp(&b.name));
    node.outputs.sort_by(|a, b| a.name.cmp(&b.name));
    for claim in &mut node.resource_claims {
        claim.network_scope.sort();
        claim.secret_scope.sort();
    }
    node.resource_claims.sort();
    entity_hash(&node)
}
fn canonical_edge_hash(edge: &crate::execution_topology::TopologyEdge) -> String {
    let mut edge = edge.clone();
    edge.resource_scope.sort();
    entity_hash(&edge)
}
fn normalize_proposal_input(input: &mut RedesignProposalInput) {
    for ids in [
        &mut input.evidence.audit_artifact_ids,
        &mut input.evidence.integration_proposal_ids,
        &mut input.evidence.expansion_proposal_ids,
        &mut input.reviewer_authority.required_capability_ids,
        &mut input.uncertainty,
        &mut input.information_loss,
    ] {
        ids.sort();
        ids.dedup();
    }
    input.expected_impact.sort_by(|a, b| {
        (&a.metric, &a.expected_direction, &a.rationale)
            .cmp(&(&b.metric, &b.expected_direction, &b.rationale))
            .then_with(|| match (a.estimated_delta, b.estimated_delta) {
                (Some(left), Some(right)) => left.total_cmp(&right),
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
            })
    });
}
fn is_artifact_id(id: &str) -> bool {
    is_prefixed_hash_id(id, "artifact:sha256-")
}
fn is_prefixed_hash_id(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix).is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
fn one(code: &str, detail: impl Into<String>) -> RedesignFinding {
    RedesignFinding {
        code: code.to_owned(),
        detail: detail.into(),
    }
}

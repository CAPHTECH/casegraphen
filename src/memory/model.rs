use crate::native_model::{CaseCell, ProjectionAudience};
use higher_graphen_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const MEMORY_SOURCE_RECORD_SCHEMA: &str = "casegraphen.experimental.memory.source_record.v0";
pub const MEMORY_CLAIM_SCHEMA: &str = "casegraphen.experimental.memory.claim.v0";
pub const MEMORY_QUERY_SCHEMA: &str = "casegraphen.experimental.memory.query.v0";
pub const MEMORY_PROJECTION_SCHEMA: &str = "casegraphen.experimental.memory.projection.v0";
pub const MEMORY_USE_REPORT_SCHEMA: &str = "casegraphen.experimental.memory.use_report.v0";
pub const MEMORY_POLICY_SCHEMA: &str = "casegraphen.experimental.memory.policy.v0";
pub const MEMORY_INDEX_SCHEMA: &str = "casegraphen.experimental.memory.index.v0";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySourceKind {
    Conversation,
    Document,
    ToolOutput,
    RuntimeTrace,
    Artifact,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityOrigin {
    User,
    Operator,
    Reviewer,
    Tool,
    External,
    Inferred,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityLevel {
    Untrusted,
    Observation,
    ProjectFact,
    ProjectConstraint,
    ProjectAuthority,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Observation,
    Episode,
    Fact,
    Constraint,
    Decision,
    Procedure,
    FailurePattern,
    Goal,
    Preference,
    Commitment,
    AuthorityStatement,
    Reference,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceRole {
    UserRequirement,
    OperatorInstruction,
    ExternalMaterial,
    ToolObservation,
    AgentInference,
    ReviewedArchitectureDecision,
    UnverifiedThirdPartyStatement,
    CanonicalHumanStatement,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Candidate,
    Accepted,
    Contested,
    Superseded,
    Expired,
    Retracted,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRecord {
    pub schema: String,
    pub source_record_id: String,
    pub source_kind: MemorySourceKind,
    pub content_hash: String,
    pub captured_at: String,
    pub origin_actor_id: String,
    pub source_boundary_id: String,
    pub authority_origin: AuthorityOrigin,
    pub sensitivity: Sensitivity,
    pub artifact_ref: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryStatement {
    pub predicate: String,
    pub object: Value,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryScope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_space_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub actor_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryClaim {
    pub schema: String,
    pub claim_id: String,
    pub memory_kind: MemoryKind,
    pub subject_refs: Vec<String>,
    pub statement: MemoryStatement,
    pub scope: MemoryScope,
    pub valid_time: ValidTime,
    pub source_refs: Vec<String>,
    pub derivation_actor_id: String,
    pub derivation_method: String,
    pub model_assertions_are_untrusted: bool,
    pub provenance_role: ProvenanceRole,
    pub authority_ceiling: AuthorityLevel,
    pub sensitivity: Sensitivity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryBudget {
    pub max_items: usize,
    pub max_tokens: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryQuery {
    pub schema: String,
    pub query_id: String,
    pub base_revision_id: String,
    pub requesting_actor_id: String,
    pub audience: ProjectionAudience,
    pub purpose: String,
    pub risk_class: String,
    pub as_of: String,
    pub scope: MemoryScope,
    pub memory_kinds: Vec<MemoryKind>,
    pub budget: MemoryBudget,
    pub query_text: String,
    #[serde(default)]
    pub include_historical: bool,
    #[serde(default)]
    pub include_contested: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorMemoryGrant {
    pub actor_id: String,
    pub allowed_audiences: Vec<ProjectionAudience>,
    pub allowed_purposes: Vec<String>,
    pub project_ids: Vec<String>,
    pub max_sensitivity: Sensitivity,
    pub max_authority: AuthorityLevel,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryPolicy {
    pub schema: String,
    pub policy_id: String,
    pub project_id: String,
    pub actor_grants: Vec<ActorMemoryGrant>,
    pub valid_time_required_kinds: Vec<MemoryKind>,
    pub hard_conflict_relation_types: Vec<String>,
    pub exact_source_escalation: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryValidationFinding {
    pub code: String,
    pub location: String,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryOmission {
    pub claim_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProjectionLoss {
    pub loss_kind: String,
    pub detail: String,
    pub omitted_claim_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedMemory {
    pub claim_id: String,
    pub memory_kind: MemoryKind,
    pub statement: MemoryStatement,
    pub subject_refs: Vec<String>,
    pub source_refs: Vec<String>,
    pub status: MemoryStatus,
    pub authority: AuthorityLevel,
    pub sensitivity: Sensitivity,
    pub valid_time: ValidTime,
    pub relevance_score: u64,
    pub evidence_strength: String,
    pub hard_conflict: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProjection {
    pub schema: String,
    pub projection_id: String,
    pub base_revision_id: String,
    pub query_hash: String,
    pub audience: ProjectionAudience,
    pub selected_claim_ids: Vec<String>,
    pub source_refs: Vec<String>,
    pub contested_claim_ids: Vec<String>,
    pub omissions: Vec<MemoryOmission>,
    pub losses: Vec<MemoryProjectionLoss>,
    pub authority_summary: BTreeMap<AuthorityLevel, usize>,
    pub temporal_cutoff: String,
    pub token_budget: usize,
    pub projection_content_hash: String,
    pub items: Vec<ProjectedMemory>,
    pub read_only: bool,
    pub accepted_state_changed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryUseReport {
    pub schema: String,
    pub projection_content_hash: String,
    pub action_id: String,
    pub cited_claim_ids: Vec<String>,
    pub ignored_constraint_ids: Vec<String>,
    pub runtime_reported_effect: String,
    pub self_reported: bool,
    pub accepted: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryIndexItem {
    pub claim_id: String,
    pub memory_kind: MemoryKind,
    pub subject_refs: Vec<String>,
    pub lexical_terms: Vec<String>,
    pub source_refs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryIndex {
    pub schema: String,
    pub base_revision_id: String,
    pub policy_id: String,
    pub query_hash: String,
    pub items: Vec<MemoryIndexItem>,
    pub index_content_hash: String,
    pub derived: bool,
    pub authoritative: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryIndexValidation {
    pub valid: bool,
    pub expected_content_hash: String,
    pub actual_content_hash: String,
    pub findings: Vec<MemoryValidationFinding>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MemoryClaimProposal {
    pub claim_cell: CaseCell,
    pub source_artifact_id: Id,
    pub findings: Vec<MemoryValidationFinding>,
    pub accepted: bool,
    pub mutation_performed: bool,
}

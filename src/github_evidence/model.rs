//! Typed owners for the `casegraphen.experimental.github.*.v0` contract family.
//!
//! One caller-authored input ([`CaptureManifest`]) and six tool-computed
//! records. The caller/tool split is the trust boundary: no record type here
//! is ever constructed by parsing a CLI input flag directly — the one narrow
//! exception (`github refresh --previous-observation` reading a
//! [`PrObservation`] back as the operator's declared review basis) still goes
//! through a recomputed content-hash check before it is trusted, which lives
//! in `refresh.rs`, not here.
//!
//! This module holds schema constants and typed records only. Normalization,
//! independence classification, refresh comparison, and projection tiering
//! are separate modules (`normalize.rs`, `independence.rs`, `refresh.rs`,
//! `projection.rs`) so each decision rule keeps exactly one implementation.

use serde::{Deserialize, Serialize};

pub const GITHUB_CAPTURE_MANIFEST_SCHEMA: &str =
    "casegraphen.experimental.github.capture_manifest.v0";
pub const GITHUB_PR_OBSERVATION_SCHEMA: &str = "casegraphen.experimental.github.pr_observation.v0";
pub const GITHUB_CHECK_EVIDENCE_SCHEMA: &str = "casegraphen.experimental.github.check_evidence.v0";
pub const GITHUB_REVIEW_FINDING_SCHEMA: &str = "casegraphen.experimental.github.review_finding.v0";
pub const GITHUB_REVIEW_INDEPENDENCE_SCHEMA: &str =
    "casegraphen.experimental.github.review_independence.v0";
pub const GITHUB_REFRESH_RESULT_SCHEMA: &str = "casegraphen.experimental.github.refresh_result.v0";
pub const GITHUB_REVIEW_PROJECTION_SCHEMA: &str =
    "casegraphen.experimental.github.review_projection.v0";

// ---------------------------------------------------------------------
// 3.1 capture_manifest.v0 (input)
// ---------------------------------------------------------------------

/// The only caller-authored file in this family. Strict-parsed: a caller
/// writing `trusted`, `approved`, `accepted`, or `authority` anywhere is
/// refused at parse, exactly like `GitHubIssueSnapshot`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureManifest {
    pub schema: String,
    pub repository: String,
    pub issue_numbers: Vec<u64>,
    pub pr_number: u64,
    pub captured_at: String,
    pub capture_tool: String,
    pub entries: Vec<CaptureEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureEntry {
    pub category: CaptureCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u64>,
    pub artifact_path: String,
    pub content_hash: String,
    pub command_record: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureCategory {
    Issue,
    Pr,
    Files,
    Reviews,
    ReviewThreads,
    Commits,
    Checks,
}

// ---------------------------------------------------------------------
// 3.2 pr_observation.v0 (record)
// ---------------------------------------------------------------------

/// The normalized, content-addressed review snapshot. Binds exact
/// repository, PR number, base SHA, and head SHA.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrObservation {
    pub schema: String,
    pub observation_id: String,
    pub repository: String,
    pub issues: Vec<PrObservationIssue>,
    pub pr: PrObservationPr,
    pub base: RefSha,
    pub head: RefSha,
    pub liveness: Liveness,
    pub changed_files: Vec<ChangedFile>,
    pub implementation_actors: ImplementationActors,
    pub source_record_ids: Vec<String>,
    pub captured_at: String,
    pub provider_fields_unmapped: bool,
    pub normalized_content_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefSha {
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub sha: String,
}

/// `pr.author` and `thread.resolved_by` share this shape: the GitHub node id
/// is the stable actor identity, `login` is display-only (§6).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrAuthor {
    pub id: String,
    pub login: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrObservationIssue {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_reason: Option<String>,
    pub url: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    pub body_content_hash: String,
    pub closed_by_pr_numbers: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrObservationPr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub author: PrAuthor,
    pub created_at: String,
    pub body_content_hash: String,
}

/// Verbatim provider strings, never mapped to booleans. `mergeable` is the
/// three-state observation GitHub actually reports (§3.2): a merged PR is
/// commonly `mergeable: UNKNOWN` because GitHub stops reporting it post-merge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Liveness {
    pub state: String,
    pub mergeable: MergeableState,
    pub merge_state_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_commit_sha: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MergeableState {
    Mergeable,
    Conflicting,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChangedFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
    pub change_type: String,
}

/// Computed from the captured `pr` + `commits` artifacts, never
/// caller-suppliable. Identity is the node id; `logins` is display-only (§6).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationActors {
    pub actor_ids: Vec<String>,
    pub logins: Vec<String>,
    pub derivation: String,
}

// ---------------------------------------------------------------------
// 3.3 check_evidence.v0 (record)
// ---------------------------------------------------------------------

/// `{id, login, typename}` verbatim from the capture — used where the
/// provider's Actor discriminator is always present together with the id
/// and login (GraphQL check/status creators).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub id: String,
    pub login: String,
    pub typename: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    CheckRun,
    StatusContext,
}

/// One per check run or commit status at the observed head.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckEvidence {
    pub schema: String,
    pub check_id: String,
    pub head_sha: String,
    pub kind: CheckKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<Actor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Schema `const: "ci_check"` — always this value for both check kinds.
    pub evidence_role: String,
    pub source_record_id: String,
}

// ---------------------------------------------------------------------
// 3.4 review_finding.v0 (record)
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    ReviewSummary,
    ThreadComment,
}

/// `id`/`typename` are each independently optional: GitHub returns an
/// attribution-less author for deleted accounts, which is a legitimate
/// provider state, not a malformed capture. A finding whose author lacks
/// them still normalizes; it classifies `unattributed` rather than being
/// refused (§6).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewFindingAuthor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub login: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typename: Option<String>,
    /// Retained as an observation; never an input to classification (§6).
    pub association: String,
}

/// `thread.resolved_by` shape. No `typename`: `resolvedBy` is GraphQL-typed
/// `User` regardless of the resolving actor's real attestation, so carrying
/// a `typename` here would invite testing a field that is an artifact of
/// the query's static type, not an attestation about the actor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedBy {
    pub id: String,
    pub login: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewThreadState {
    pub thread_id: String,
    pub resolved: bool,
    pub outdated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<ResolvedBy>,
    pub comment_count: u64,
}

/// One per review summary and per review-thread comment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewFinding {
    pub schema: String,
    pub finding_id: String,
    pub kind: FindingKind,
    pub author: ReviewFindingAuthor,
    pub authored_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_edited_at: Option<String>,
    pub edited: bool,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    pub body_content_hash: String,
    pub actionable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<ReviewThreadState>,
    pub duplicate_count: u32,
    pub source_record_id: String,
}

// ---------------------------------------------------------------------
// 3.5 review_independence.v0 (record) — never a CLI input.
// ---------------------------------------------------------------------

/// Closed, total classification of what an observation subject's review
/// role is. The implementation-actor arm precedes the attestation arms, so
/// an implementation actor can never reach `IndependentHumanCandidate`.
/// `Unattributed` is a legitimate fifth arm (missing discriminator/id on a
/// finding author), not a hard refusal, and it can never satisfy an
/// independent-review policy (§6).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRole {
    SelfReview,
    AutomatedBot,
    CiCheck,
    IndependentHumanCandidate,
    Unattributed,
}

/// Which rule in the classifier fired. Bot attestation is drawn from an
/// ordered, closed list (typename, then the provider-issued `BOT_` id
/// prefix, then id-equality with an actor already bot-attested elsewhere in
/// the same capture) — never a name heuristic (§6).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationBasis {
    CheckObservation,
    AuthorInImplementationActorSet,
    ProviderBotDiscriminator,
    ProviderBotIdPrefix,
    ProviderBotIdEquality,
    ProviderUserDiscriminator,
    AttestationAbsent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Classification {
    pub subject_id: String,
    pub evidence_role: EvidenceRole,
    pub basis: ClassificationBasis,
}

/// An `APPROVED` review whose `commit_sha` does not equal the observed head
/// — excluded and visibly recorded rather than credited by fallback (§3.5).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExcludedApproval {
    pub finding_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IndependencePolicy {
    pub require_independent_review: bool,
    pub satisfied: bool,
    pub satisfying_finding_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IndependenceFinding {
    pub code: String,
    pub detail: String,
}

/// The independence classification and policy evaluation, bound to one
/// exact `pr_observation`. `independence_proven` is schema `const: false`:
/// the record type cannot express proven independence, mirroring
/// `VerificationPolicyResult.independent_minds_proven`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewIndependence {
    pub schema: String,
    pub pr_observation_hash: String,
    pub implementation_actor_ids: Vec<String>,
    pub implementation_actor_logins: Vec<String>,
    pub classifications: Vec<Classification>,
    pub unresolved_actionable_finding_ids: Vec<String>,
    pub resolved_actionable_finding_ids: Vec<String>,
    pub independent_human_approvals: Vec<String>,
    pub excluded_approvals: Vec<ExcludedApproval>,
    pub policy: IndependencePolicy,
    pub independence_proven: bool,
    pub findings: Vec<IndependenceFinding>,
}

// ---------------------------------------------------------------------
// 3.6 refresh_result.v0 (record)
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshDisposition {
    HeadUnchanged,
    StaleHead,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationChangeKind {
    Added,
    Removed,
    Changed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationChange {
    pub category: String,
    pub change: ObservationChangeKind,
    pub subject_id: String,
    pub detail: String,
}

/// `review_basis_moved` is schema `const: false` — a refresh cannot rebase
/// by construction. `refreshed_observation_hash` is present only when
/// `disposition == head_unchanged`; a stale-head refresh emits no new
/// observation (§7).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshResult {
    pub schema: String,
    pub previous_observation_hash: String,
    pub previous_head_sha: String,
    pub previous_base_sha: String,
    pub observed_head_sha: String,
    pub observed_base_sha: String,
    pub disposition: RefreshDisposition,
    pub review_basis_moved: bool,
    pub observation_changes: Vec<ObservationChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_observation_hash: Option<String>,
}

// ---------------------------------------------------------------------
// 3.7 review_projection.v0 (record)
// ---------------------------------------------------------------------

/// Not a wire field of `ReviewProjection` (which carries three separate
/// arrays instead) — a helper vocabulary for `projection.rs`'s tier rule to
/// tag a subject before it is placed into `must_review` / `should_review` /
/// `can_skim`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTier {
    MustReview,
    ShouldReview,
    CanSkim,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TierItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub subject_ids: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindingReason {
    pub finding_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationSource {
    pub subject_id: String,
    pub evidence_role: EvidenceRole,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualRisk {
    pub code: String,
    pub detail: String,
}

/// Never empty in v0 — bodies are always hashed out (§8).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionLoss {
    pub loss_kind: String,
    pub detail: String,
    pub omitted_refs: Vec<String>,
}

/// The separately available full audit trace a compact projection cites
/// but never replaces.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FullTrace {
    pub source_record_ids: Vec<String>,
    pub pr_observation_hash: String,
    pub check_ids: Vec<String>,
    pub finding_ids: Vec<String>,
    pub independence_included: bool,
}

/// The compact reviewer projection: Must/Should/Can tiers, blocking vs.
/// non-blocking findings, declared loss, and a full-trace citation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewProjection {
    pub schema: String,
    pub projection_id: String,
    pub pr_observation_hash: String,
    pub repository: String,
    pub pr_number: u64,
    pub base_sha: String,
    pub head_sha: String,
    pub liveness: Liveness,
    pub must_review: Vec<TierItem>,
    pub should_review: Vec<TierItem>,
    pub can_skim: Vec<TierItem>,
    pub blocking_findings: Vec<FindingReason>,
    pub non_blocking_findings: Vec<FindingReason>,
    pub unresolved_threads: Vec<String>,
    pub failed_checks: Vec<String>,
    pub inconclusive_checks: Vec<String>,
    pub verification_sources: Vec<VerificationSource>,
    pub residual_risks: Vec<ResidualRisk>,
    pub losses: Vec<ProjectionLoss>,
    pub full_trace: FullTrace,
    pub projection_content_hash: String,
    pub read_only: bool,
    pub accepted: bool,
}

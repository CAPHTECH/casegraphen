//! Experimental verification-policy reconciliation.
//!
//! This module distinguishes ledger-observable identity/capability facts,
//! runtime attestations, and properties CaseGraphen cannot observe. A policy
//! result is not an evidence acceptance or a proof of independent minds.

use crate::{
    exec::records::{
        ExecutionDispatchState, ExecutionTrace, EXECUTION_RECORD_SCHEMA_VERSION,
        EXECUTION_TRACE_SCHEMA,
    },
    native_eval::{
        evaluate_native_case, latest_evidence_review_entries, latest_evidence_review_status,
    },
    native_hash::sha256_hex,
    native_model::{CaseCellLifecycle, CaseCellType, CaseMorphismType, CaseSpace, ReviewAction},
    native_review::{canonical_review, check_operation_gate, NativeOperationGate},
    runtime_protocol::{
        validate_runtime_node_report, RuntimeNodeReport, RUNTIME_NODE_REPORT_SCHEMA,
        RUNTIME_NODE_REPORT_SCHEMA_VERSION,
    },
};
use higher_graphen_core::ReviewStatus;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const VERIFICATION_POLICY_SCHEMA: &str = "casegraphen.experimental.verification_policy.v0";
pub const VERIFICATION_LINEAGE_DECLARATIONS_SCHEMA: &str =
    "casegraphen.experimental.verification_lineage_declarations.v0";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimLevel {
    LedgerVerifiable,
    RuntimeAttested,
    NotObservableHere,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityConstraints {
    pub capability_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationQuorum {
    pub minimum_accepts: u32,
    pub total_verifiers: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyProvenance {
    pub source: String,
    pub created_by: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationPolicy {
    pub schema: String,
    pub verification_policy_id: String,
    pub producer_constraints: CapabilityConstraints,
    pub verifier_constraints: CapabilityConstraints,
    pub actor_must_differ: bool,
    pub lenses: Vec<String>,
    pub quorum: VerificationQuorum,
    pub required_anchors: Vec<String>,
    pub allowed_runtime_attestations: Vec<String>,
    pub provenance: PolicyProvenance,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierDisposition {
    Accept,
    Reject,
    Abstain,
}

/// Caller-declared producer identity. This remains untrusted integration input
/// and cannot satisfy ledger-derived verification requirements.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredProducerLineage {
    pub actor_id: String,
    pub capability_ids: Vec<String>,
}

/// Caller-declared verifier identity and disposition. It is intentionally not
/// a ledger proof, even when its identifiers happen to name ledger objects.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredVerifierRecord {
    pub verifier_report_id: String,
    pub actor_id: String,
    pub capability_ids: Vec<String>,
    pub disposition: VerifierDisposition,
    pub runtime_attestations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationLineageDeclarations {
    pub schema: String,
    pub schema_version: u32,
    pub producer: DeclaredProducerLineage,
    pub verifiers: Vec<DeclaredVerifierRecord>,
}

/// Shared content and revision boundary carried by opaque lineage proofs.
#[derive(Clone, Debug, Eq, PartialEq)]
struct LedgerLineageBinding {
    case_space_id: String,
    observed_case_revision_id: String,
    /// Deployment/subject revision shared by producer and verifier proofs.
    /// This comes from the content-addressed execution trace, not from the
    /// later ledger revision that records each authority morphism.
    case_revision_id: String,
    claim_cell_id: String,
    topology_content_hash: String,
    node_id: String,
    attempt_id: String,
    actor_id: String,
    capability_ids: Vec<String>,
    operation_gate: NativeOperationGate,
    operation_gate_content_hash: String,
    runtime_report_content_hash: String,
    execution_trace_content_hash: String,
    authority_morphism_id: String,
}

/// Opaque proof that a producer identity was derived from the exact current
/// ledger revision, a canonical operation gate/capability grant, an accepted
/// dispatch morphism, and exact runtime-report/trace bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerDerivedProducerProof {
    binding: LedgerLineageBinding,
}

/// Opaque proof that a verifier identity and disposition were derived from the
/// exact current ledger revision, a canonical operation gate/capability grant,
/// an accepted review morphism, and exact verifier-report/trace bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerDerivedVerifierProof {
    binding: LedgerLineageBinding,
    verifier_report_id: String,
    disposition: VerifierDisposition,
    runtime_attestations: Vec<String>,
}

/// Explicit binding requested when deriving producer or verifier authority.
/// Every field is checked against ledger-owned records; it is not itself a
/// proof or an authorization object.
pub struct LedgerLineageDerivation<'a> {
    pub case_space: &'a CaseSpace,
    pub claim_cell_id: &'a str,
    pub topology_content_hash: &'a str,
    pub node_id: &'a str,
    pub attempt_id: &'a str,
    pub operation_gate: &'a NativeOperationGate,
    pub authority_morphism_id: &'a str,
    pub runtime_report_bytes: &'a [u8],
    pub execution_trace_bytes: &'a [u8],
}

/// Caller-declared anchor metadata. It is intentionally reconciled under
/// declaration-only vocabulary and can never satisfy a tool-observed policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeclaredAnchorObservation {
    SourceArtifactHash {
        anchor_id: String,
        expected_sha256: String,
        observed_sha256: String,
    },
    ToolObservedTest {
        anchor_id: String,
        command_hash: String,
        exit_code: i32,
    },
}

impl DeclaredAnchorObservation {
    fn id(&self) -> &str {
        match self {
            Self::SourceArtifactHash { anchor_id, .. }
            | Self::ToolObservedTest { anchor_id, .. } => anchor_id,
        }
    }

    fn declaration_matches(&self) -> bool {
        match self {
            Self::SourceArtifactHash {
                expected_sha256,
                observed_sha256,
                ..
            } => is_sha256(expected_sha256) && expected_sha256 == observed_sha256,
            Self::ToolObservedTest {
                command_hash,
                exit_code,
                ..
            } => is_sha256(command_hash) && *exit_code == 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AnchorProofProvenance {
    CaseArtifact {
        case_space_id: String,
        case_revision_id: String,
        artifact_id: String,
    },
    CaseExecutionTrace {
        case_space_id: String,
        case_revision_id: String,
        trace_id: String,
        trace_content_hash: String,
    },
    TrustedReferenceAdapter {
        adapter_id: String,
        artifact_id: String,
    },
}

/// Opaque proof that CaseGraphen or a crate-trusted reference adapter observed
/// exact bytes. Normal evidence review remains the only acceptance authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolObservedAnchorProof {
    anchor_id: String,
    observed_content_hash: String,
    provenance: AnchorProofProvenance,
}

impl ToolObservedAnchorProof {
    fn id(&self) -> &str {
        &self.anchor_id
    }

    fn provenance_is_well_formed(&self) -> bool {
        is_sha256(&self.observed_content_hash)
            && match &self.provenance {
                AnchorProofProvenance::CaseArtifact {
                    case_space_id,
                    case_revision_id,
                    artifact_id,
                } => {
                    !case_space_id.is_empty()
                        && !case_revision_id.is_empty()
                        && artifact_id == &format!("artifact:sha256-{}", self.observed_content_hash)
                }
                AnchorProofProvenance::CaseExecutionTrace {
                    case_space_id,
                    case_revision_id,
                    trace_id,
                    trace_content_hash,
                } => {
                    !case_space_id.is_empty()
                        && !case_revision_id.is_empty()
                        && !trace_id.is_empty()
                        && trace_content_hash == &self.observed_content_hash
                }
                AnchorProofProvenance::TrustedReferenceAdapter {
                    adapter_id,
                    artifact_id,
                } => {
                    !adapter_id.is_empty()
                        && artifact_id == &format!("artifact:sha256-{}", self.observed_content_hash)
                }
            }
    }
}

/// Exact files that an anchored CaseGraphen execution trace commits to.
pub struct AnchoredExecutionTraceBytes<'a> {
    pub trace: &'a [u8],
    pub worker_report: &'a [u8],
    pub stdout: &'a [u8],
    pub stderr: &'a [u8],
}

/// Capability held only by an explicitly trusted in-crate reference adapter.
/// External callers cannot construct it or promote runtime declarations.
pub struct TrustedReferenceAnchorAdapter {
    adapter_id: String,
}

impl TrustedReferenceAnchorAdapter {
    #[allow(dead_code)]
    pub(crate) fn new(adapter_id: impl Into<String>) -> Self {
        Self {
            adapter_id: adapter_id.into(),
        }
    }
}

/// Derives a proof only when a canonical CaseGraphen space contains the exact
/// content-addressed artifact cell and the supplied bytes match it.
pub fn observe_case_artifact(
    case_space: &CaseSpace,
    anchor_id: &str,
    artifact_id: &str,
    artifact_bytes: &[u8],
) -> Result<ToolObservedAnchorProof, Vec<PolicyFinding>> {
    let mut findings = validate_observation_identity(anchor_id);
    if evaluate_native_case(case_space).is_err() {
        findings.push(finding(
            "case_artifact_space_invalid",
            ClaimLevel::LedgerVerifiable,
            Some(anchor_id.to_owned()),
            "artifact observation requires a canonically valid CaseGraphen space",
        ));
    }
    let content_hash = sha256_hex(artifact_bytes);
    let expected_id = format!("artifact:sha256-{content_hash}");
    let cell_matches = case_space.case_cells.iter().any(|cell| {
        cell.id.as_str() == artifact_id
            && cell.space_id == case_space.space_id
            && matches!(&cell.cell_type, CaseCellType::Custom(kind) if kind == "artifact")
            && matches!(
                cell.lifecycle,
                CaseCellLifecycle::Resolved | CaseCellLifecycle::Accepted
            )
            && cell
                .metadata
                .get("content_hash")
                .and_then(serde_json::Value::as_str)
                == Some(content_hash.as_str())
    });
    if artifact_id != expected_id || !cell_matches {
        findings.push(finding(
            "case_artifact_content_mismatch",
            ClaimLevel::LedgerVerifiable,
            Some(anchor_id.to_owned()),
            "artifact id, ledger cell, metadata hash, and observed bytes must match exactly",
        ));
    }
    if findings.is_empty() {
        Ok(ToolObservedAnchorProof {
            anchor_id: anchor_id.to_owned(),
            observed_content_hash: content_hash,
            provenance: AnchorProofProvenance::CaseArtifact {
                case_space_id: case_space.case_space_id.to_string(),
                case_revision_id: case_space.revision.revision_id.to_string(),
                artifact_id: artifact_id.to_owned(),
            },
        })
    } else {
        Err(findings)
    }
}

/// Derives a proof only from a trace that is content-bound by an accepted
/// CaseGraphen execution-trace anchor and whose committed output bytes match.
pub fn observe_case_execution_trace(
    case_space: &CaseSpace,
    anchor_id: &str,
    files: AnchoredExecutionTraceBytes<'_>,
) -> Result<ToolObservedAnchorProof, Vec<PolicyFinding>> {
    let mut findings = validate_observation_identity(anchor_id);
    if evaluate_native_case(case_space).is_err() {
        findings.push(finding(
            "case_trace_space_invalid",
            ClaimLevel::LedgerVerifiable,
            Some(anchor_id.to_owned()),
            "trace observation requires a canonically valid CaseGraphen space",
        ));
    }
    let trace = match serde_json::from_slice::<ExecutionTrace>(files.trace) {
        Ok(trace) => Some(trace),
        Err(error) => {
            findings.push(finding(
                "invalid_execution_trace_bytes",
                ClaimLevel::LedgerVerifiable,
                Some(anchor_id.to_owned()),
                error.to_string(),
            ));
            None
        }
    };
    let trace_hash = sha256_hex(files.trace);
    if let Some(trace) = &trace {
        let schema_valid = trace.schema == EXECUTION_TRACE_SCHEMA
            && trace.schema_version == EXECUTION_RECORD_SCHEMA_VERSION
            && trace.dispatch_state == ExecutionDispatchState::Completed
            && trace.case_space_id == case_space.case_space_id
            && trace.operation_gate.operation_scope_id == case_space.case_space_id;
        let outputs_valid = sha256_hex(files.worker_report) == trace.worker_report_content_hash
            && sha256_hex(files.stdout) == trace.stdout_content_hash
            && sha256_hex(files.stderr) == trace.stderr_content_hash;
        let anchor_entry = case_space.morphism_log.iter().find(|entry| {
            entry.morphism.morphism_type
                == CaseMorphismType::Custom("execution_trace_anchor".to_owned())
                && entry.morphism.review_status == ReviewStatus::Accepted
                && entry
                    .morphism
                    .metadata
                    .get("trace_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(trace.trace_id.as_str())
                && entry
                    .morphism
                    .metadata
                    .get("trace_content_hash")
                    .and_then(serde_json::Value::as_str)
                    == Some(trace_hash.as_str())
        });
        let ledger_join_valid = anchor_entry.is_some_and(|entry| {
            trace.appended_entry_ids.contains(&entry.entry_id)
                && trace.result_revision_id.as_ref() == Some(&entry.target_revision_id)
        });
        if !schema_valid || !outputs_valid || !ledger_join_valid {
            findings.push(finding(
                "execution_trace_provenance_mismatch",
                ClaimLevel::LedgerVerifiable,
                Some(anchor_id.to_owned()),
                "trace schema, anchored identity, result revision, and committed output bytes must all match",
            ));
        }
    }
    if findings.is_empty() {
        let trace = trace.expect("valid observation retained parsed trace");
        Ok(ToolObservedAnchorProof {
            anchor_id: anchor_id.to_owned(),
            observed_content_hash: trace_hash.clone(),
            provenance: AnchorProofProvenance::CaseExecutionTrace {
                case_space_id: case_space.case_space_id.to_string(),
                case_revision_id: case_space.revision.revision_id.to_string(),
                trace_id: trace.trace_id.to_string(),
                trace_content_hash: trace_hash,
            },
        })
    } else {
        Err(findings)
    }
}

/// Crate-trusted deterministic reference adapter. Requiring exact bytes (not a
/// caller-supplied observed hash) prevents a self-report from copying hashes
/// into the stronger proof type.
pub fn observe_trusted_reference_artifact(
    adapter: &TrustedReferenceAnchorAdapter,
    anchor_id: &str,
    artifact_id: &str,
    artifact_bytes: &[u8],
) -> Result<ToolObservedAnchorProof, Vec<PolicyFinding>> {
    let mut findings = validate_observation_identity(anchor_id);
    let content_hash = sha256_hex(artifact_bytes);
    if adapter.adapter_id.is_empty() || artifact_id != format!("artifact:sha256-{content_hash}") {
        findings.push(finding(
            "trusted_reference_artifact_mismatch",
            ClaimLevel::LedgerVerifiable,
            Some(anchor_id.to_owned()),
            "trusted adapter identity and exact content-addressed bytes are required",
        ));
    }
    if findings.is_empty() {
        Ok(ToolObservedAnchorProof {
            anchor_id: anchor_id.to_owned(),
            observed_content_hash: content_hash,
            provenance: AnchorProofProvenance::TrustedReferenceAdapter {
                adapter_id: adapter.adapter_id.clone(),
                artifact_id: artifact_id.to_owned(),
            },
        })
    } else {
        Err(findings)
    }
}

fn validate_observation_identity(anchor_id: &str) -> Vec<PolicyFinding> {
    if anchor_id.is_empty() {
        vec![finding(
            "empty_observed_anchor_id",
            ClaimLevel::LedgerVerifiable,
            None,
            "tool-observed anchor id must not be empty",
        )]
    } else {
        Vec::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeclaredAnchorReconciliation {
    pub policy_id: String,
    pub declared_anchors_match: bool,
    pub findings: Vec<PolicyFinding>,
}

pub fn reconcile_declared_anchors(
    policy: &VerificationPolicy,
    declarations: &[DeclaredAnchorObservation],
) -> DeclaredAnchorReconciliation {
    let mut findings = validate_verification_policy(policy);
    let mut declarations_by_id = std::collections::BTreeMap::new();
    for declaration in declarations {
        if declaration.id().is_empty()
            || declarations_by_id
                .insert(declaration.id(), declaration)
                .is_some()
        {
            findings.push(finding(
                "duplicate_or_empty_declared_anchor_id",
                ClaimLevel::RuntimeAttested,
                Some(declaration.id().to_owned()),
                "declared anchor ids must be unique and non-empty",
            ));
        }
    }
    let declared_anchors_match = policy.required_anchors.iter().all(|required| {
        declarations_by_id
            .get(required.as_str())
            .is_some_and(|declaration| declaration.declaration_matches())
    });
    if !declared_anchors_match {
        findings.push(finding(
            "declared_anchor_mismatch",
            ClaimLevel::RuntimeAttested,
            None,
            "one or more caller-declared anchors are absent or internally inconsistent",
        ));
    }
    findings.sort_by(|left, right| {
        (&left.code, &left.subject_id).cmp(&(&right.code, &right.subject_id))
    });
    DeclaredAnchorReconciliation {
        policy_id: policy.verification_policy_id.clone(),
        declared_anchors_match,
        findings,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PolicyFinding {
    pub code: String,
    pub level: ClaimLevel,
    pub subject_id: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LedgerDerivedLineageScope {
    pub case_space_id: String,
    pub case_revision_id: String,
    pub claim_cell_id: String,
    pub topology_content_hash: String,
    pub node_id: String,
    pub attempt_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VerificationPolicyResult {
    pub policy_id: String,
    pub ledger_scope: LedgerDerivedLineageScope,
    pub ledger_requirements_satisfied: bool,
    pub runtime_attestations_satisfied: bool,
    pub anchors_satisfied: bool,
    pub quorum_satisfied: bool,
    pub policy_satisfied: bool,
    pub independent_minds_proven: bool,
    pub fresh_context_proven: bool,
    pub findings: Vec<PolicyFinding>,
}

pub fn parse_verification_policy(input: &str) -> Result<VerificationPolicy, Vec<PolicyFinding>> {
    let policy: VerificationPolicy = serde_json::from_str(input).map_err(|error| {
        vec![finding(
            "invalid_json",
            ClaimLevel::LedgerVerifiable,
            None,
            error.to_string(),
        )]
    })?;
    let findings = validate_verification_policy(&policy);
    if findings.is_empty() {
        Ok(policy)
    } else {
        Err(findings)
    }
}

pub fn validate_verification_policy(policy: &VerificationPolicy) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    if policy.schema != VERIFICATION_POLICY_SCHEMA {
        findings.push(finding(
            "unsupported_schema",
            ClaimLevel::LedgerVerifiable,
            None,
            "schema identity does not match verification_policy.v0",
        ));
    }
    for (field, value) in [
        (
            "verification_policy_id",
            policy.verification_policy_id.as_str(),
        ),
        ("provenance.source", policy.provenance.source.as_str()),
        (
            "provenance.created_by",
            policy.provenance.created_by.as_str(),
        ),
    ] {
        if value.is_empty() {
            findings.push(finding(
                "empty_required_field",
                ClaimLevel::LedgerVerifiable,
                None,
                format!("{field} must not be empty"),
            ));
        }
    }
    if policy.quorum.minimum_accepts == 0
        || policy.quorum.total_verifiers == 0
        || policy.quorum.minimum_accepts > policy.quorum.total_verifiers
    {
        findings.push(finding(
            "invalid_quorum",
            ClaimLevel::LedgerVerifiable,
            None,
            "quorum must satisfy 1 <= minimum_accepts <= total_verifiers",
        ));
    }
    for (field, values) in [
        (
            "producer capability",
            &policy.producer_constraints.capability_ids,
        ),
        (
            "verifier capability",
            &policy.verifier_constraints.capability_ids,
        ),
        ("lens", &policy.lenses),
        ("anchor", &policy.required_anchors),
        ("runtime attestation", &policy.allowed_runtime_attestations),
    ] {
        let mut seen = BTreeSet::new();
        for value in values {
            if value.is_empty() || !seen.insert(value) {
                findings.push(finding(
                    "invalid_policy_identifier",
                    ClaimLevel::LedgerVerifiable,
                    None,
                    format!("{field} values must be non-empty and unique"),
                ));
            }
        }
    }
    findings
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeclaredLineageReconciliation {
    pub policy_id: String,
    pub declarations_well_formed: bool,
    pub ledger_requirements_satisfied: bool,
    pub findings: Vec<PolicyFinding>,
}

/// Validates caller declarations without promoting actor, capability,
/// disposition, or quorum claims to ledger facts.
pub fn reconcile_declared_lineage(
    policy: &VerificationPolicy,
    producer: &DeclaredProducerLineage,
    verifiers: &[DeclaredVerifierRecord],
) -> DeclaredLineageReconciliation {
    let mut findings = validate_verification_policy(policy);
    if producer.actor_id.is_empty() || producer.capability_ids.iter().any(String::is_empty) {
        findings.push(finding(
            "invalid_declared_producer_lineage",
            ClaimLevel::RuntimeAttested,
            Some(producer.actor_id.clone()),
            "declared producer actor and capability ids must be non-empty",
        ));
    }
    let mut report_ids = BTreeSet::new();
    for verifier in verifiers {
        if verifier.verifier_report_id.is_empty()
            || verifier.actor_id.is_empty()
            || verifier.capability_ids.iter().any(String::is_empty)
            || !report_ids.insert(verifier.verifier_report_id.as_str())
        {
            findings.push(finding(
                "invalid_declared_verifier_lineage",
                ClaimLevel::RuntimeAttested,
                Some(verifier.verifier_report_id.clone()),
                "declared verifier identities must be unique and non-empty",
            ));
        }
    }
    findings.push(finding(
        "declared_lineage_not_ledger_authority",
        ClaimLevel::RuntimeAttested,
        None,
        "caller-declared actor, capability, disposition, and quorum values cannot satisfy ledger requirements",
    ));
    findings.sort_by(|left, right| {
        (&left.code, &left.subject_id).cmp(&(&right.code, &right.subject_id))
    });
    let declarations_well_formed = !findings.iter().any(|finding| {
        matches!(
            finding.code.as_str(),
            "invalid_declared_producer_lineage" | "invalid_declared_verifier_lineage"
        )
    });
    DeclaredLineageReconciliation {
        policy_id: policy.verification_policy_id.clone(),
        declarations_well_formed,
        ledger_requirements_satisfied: false,
        findings,
    }
}

fn lineage_metadata_matches(
    metadata: &serde_json::Map<String, serde_json::Value>,
    binding: &LedgerLineageBinding,
) -> bool {
    [
        ("case_revision_id", binding.case_revision_id.as_str()),
        ("claim_cell_id", binding.claim_cell_id.as_str()),
        (
            "topology_content_hash",
            binding.topology_content_hash.as_str(),
        ),
        ("node_id", binding.node_id.as_str()),
        ("attempt_id", binding.attempt_id.as_str()),
        (
            "operation_gate_content_hash",
            binding.operation_gate_content_hash.as_str(),
        ),
        (
            "runtime_report_content_hash",
            binding.runtime_report_content_hash.as_str(),
        ),
        (
            "execution_trace_content_hash",
            binding.execution_trace_content_hash.as_str(),
        ),
    ]
    .into_iter()
    .all(|(key, expected)| metadata.get(key).and_then(serde_json::Value::as_str) == Some(expected))
}

fn lineage_authority_matches(
    entry: &crate::native_model::MorphismLogEntry,
    binding: &LedgerLineageBinding,
    expected_morphism: &str,
    trace: Option<&ExecutionTrace>,
) -> bool {
    if entry.actor_id.as_str() != binding.actor_id
        || entry.morphism.review_status != ReviewStatus::Accepted
        || !lineage_metadata_matches(&entry.morphism.metadata, binding)
    {
        return false;
    }
    if expected_morphism == "review" {
        entry.morphism.morphism_type == CaseMorphismType::Review
            && canonical_review(&entry.morphism).is_some_and(|review| {
                matches!(review.action, ReviewAction::Accept | ReviewAction::Reject)
                    && matches!(
                        review.outcome,
                        ReviewStatus::Accepted | ReviewStatus::Rejected
                    )
            })
            && match trace {
                None => true,
                Some(trace) => {
                    Some(&trace.base_revision_id) == entry.source_revision_id.as_ref()
                        && trace.result_revision_id.as_ref() == Some(&entry.target_revision_id)
                        && trace.appended_entry_ids.contains(&entry.entry_id)
                }
            }
    } else if expected_morphism == "execution_trace_anchor" {
        entry.morphism.morphism_type
            == CaseMorphismType::Custom("execution_trace_anchor".to_owned())
            && match trace {
                None => true,
                Some(trace) => {
                    trace.result_revision_id.as_ref() == Some(&entry.target_revision_id)
                        && trace.appended_entry_ids.contains(&entry.entry_id)
                        && entry
                            .morphism
                            .metadata
                            .get("trace_id")
                            .and_then(serde_json::Value::as_str)
                            == Some(trace.trace_id.as_str())
                        && entry
                            .morphism
                            .metadata
                            .get("trace_content_hash")
                            .and_then(serde_json::Value::as_str)
                            == Some(binding.execution_trace_content_hash.as_str())
                }
            }
    } else {
        false
    }
}

fn derive_lineage_binding(
    input: &LedgerLineageDerivation<'_>,
    expected_operation: &str,
    expected_morphism: &str,
    subject_revision_id: Option<&str>,
) -> Result<(LedgerLineageBinding, RuntimeNodeReport), Vec<PolicyFinding>> {
    let mut findings = Vec::new();
    if evaluate_native_case(input.case_space).is_err() {
        findings.push(finding(
            "lineage_case_space_invalid",
            ClaimLevel::LedgerVerifiable,
            None,
            "lineage proof requires a canonically valid CaseGraphen space",
        ));
    }
    if let Err(error) =
        check_operation_gate(input.case_space, input.operation_gate, expected_operation)
    {
        findings.push(finding(
            "lineage_operation_gate_invalid",
            ClaimLevel::LedgerVerifiable,
            Some(input.operation_gate.actor_id.to_string()),
            error.to_string(),
        ));
    }
    let report = match serde_json::from_slice::<RuntimeNodeReport>(input.runtime_report_bytes) {
        Ok(report) => {
            for report_finding in validate_runtime_node_report(&report) {
                findings.push(finding(
                    "lineage_runtime_report_invalid",
                    ClaimLevel::LedgerVerifiable,
                    Some(report.report_id.clone()),
                    format!("{}: {}", report_finding.code, report_finding.detail),
                ));
            }
            Some(report)
        }
        Err(error) => {
            findings.push(finding(
                "lineage_runtime_report_invalid",
                ClaimLevel::LedgerVerifiable,
                None,
                error.to_string(),
            ));
            None
        }
    };
    let trace = match serde_json::from_slice::<ExecutionTrace>(input.execution_trace_bytes) {
        Ok(trace) => Some(trace),
        Err(error) => {
            findings.push(finding(
                "lineage_execution_trace_invalid",
                ClaimLevel::LedgerVerifiable,
                None,
                error.to_string(),
            ));
            None
        }
    };
    let operation_gate_content_hash = sha256_hex(
        &serde_json::to_vec(input.operation_gate)
            .expect("operation gate serialization cannot fail"),
    );
    let binding = LedgerLineageBinding {
        case_space_id: input.case_space.case_space_id.to_string(),
        observed_case_revision_id: input.case_space.revision.revision_id.to_string(),
        case_revision_id: subject_revision_id
            .map(ToOwned::to_owned)
            .or_else(|| {
                trace
                    .as_ref()
                    .map(|trace| trace.base_revision_id.to_string())
            })
            .unwrap_or_default(),
        claim_cell_id: input.claim_cell_id.to_owned(),
        topology_content_hash: input.topology_content_hash.to_owned(),
        node_id: input.node_id.to_owned(),
        attempt_id: input.attempt_id.to_owned(),
        actor_id: input.operation_gate.actor_id.to_string(),
        capability_ids: input
            .operation_gate
            .capability_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
        operation_gate: input.operation_gate.clone(),
        operation_gate_content_hash,
        runtime_report_content_hash: sha256_hex(input.runtime_report_bytes),
        execution_trace_content_hash: sha256_hex(input.execution_trace_bytes),
        authority_morphism_id: input.authority_morphism_id.to_owned(),
    };
    let claim_matches = input.case_space.case_cells.iter().any(|cell| {
        cell.id.as_str() == input.claim_cell_id
            && cell
                .metadata
                .get("execution_topology_content_hash")
                .and_then(serde_json::Value::as_str)
                == Some(input.topology_content_hash)
    });
    if !claim_matches {
        findings.push(finding(
            "lineage_claim_binding_mismatch",
            ClaimLevel::LedgerVerifiable,
            Some(input.claim_cell_id.to_owned()),
            "claim cell and exact topology content hash must exist in the current ledger revision",
        ));
    }
    if let Some(report) = &report {
        if report.schema != RUNTIME_NODE_REPORT_SCHEMA
            || report.schema_version != RUNTIME_NODE_REPORT_SCHEMA_VERSION
            || report.runtime_graph_content_hash != input.topology_content_hash
            || report.node_id != input.node_id
            || report.attempt_id != input.attempt_id
        {
            findings.push(finding(
                "lineage_runtime_binding_mismatch",
                ClaimLevel::LedgerVerifiable,
                Some(report.report_id.clone()),
                "runtime report must match the exact topology, node, and attempt binding",
            ));
        }
    }
    if let Some(trace) = &trace {
        if trace.schema != EXECUTION_TRACE_SCHEMA
            || trace.schema_version != EXECUTION_RECORD_SCHEMA_VERSION
            || trace.case_space_id != input.case_space.case_space_id
            || trace.operation_gate != *input.operation_gate
            || trace.dispatch_state != ExecutionDispatchState::Completed
        {
            findings.push(finding(
                "lineage_trace_binding_mismatch",
                ClaimLevel::LedgerVerifiable,
                Some(trace.trace_id.to_string()),
                "execution trace must bind the exact case space, canonical gate, and completed dispatch",
            ));
        }
    }
    let authority_entry = input.case_space.morphism_log.iter().find(|entry| {
        entry.morphism_id.as_str() == input.authority_morphism_id
            && entry.actor_id == input.operation_gate.actor_id
            && entry.morphism.review_status == ReviewStatus::Accepted
    });
    let authority_matches = authority_entry.is_some_and(|entry| {
        lineage_authority_matches(entry, &binding, expected_morphism, trace.as_ref())
    });
    if !authority_matches {
        findings.push(finding(
            "lineage_authority_morphism_mismatch",
            ClaimLevel::LedgerVerifiable,
            Some(input.authority_morphism_id.to_owned()),
            "an accepted canonical dispatch/review morphism must bind the shared subject revision and every lineage content hash",
        ));
    }
    if findings.is_empty() {
        Ok((
            binding,
            report.expect("valid lineage retained runtime report"),
        ))
    } else {
        Err(findings)
    }
}

pub fn derive_ledger_producer_proof(
    input: LedgerLineageDerivation<'_>,
) -> Result<LedgerDerivedProducerProof, Vec<PolicyFinding>> {
    derive_lineage_binding(&input, "dispatch", "execution_trace_anchor", None)
        .map(|(binding, _)| LedgerDerivedProducerProof { binding })
}

pub fn derive_ledger_verifier_proof(
    input: LedgerLineageDerivation<'_>,
    producer: &LedgerDerivedProducerProof,
) -> Result<LedgerDerivedVerifierProof, Vec<PolicyFinding>> {
    let (binding, report) = derive_lineage_binding(
        &input,
        "review",
        "review",
        Some(&producer.binding.case_revision_id),
    )?;
    if binding.case_space_id != producer.binding.case_space_id
        || binding.claim_cell_id != producer.binding.claim_cell_id
        || binding.topology_content_hash != producer.binding.topology_content_hash
        || binding.node_id != producer.binding.node_id
        || binding.attempt_id != producer.binding.attempt_id
    {
        return Err(vec![finding(
            "verifier_lineage_binding_mismatch",
            ClaimLevel::LedgerVerifiable,
            Some(report.report_id.clone()),
            "verifier proof must bind the canonical producer subject, claim, topology, node, and attempt",
        )]);
    }
    let entry = input
        .case_space
        .morphism_log
        .iter()
        .find(|entry| entry.morphism_id.as_str() == input.authority_morphism_id)
        .expect("validated verifier authority entry");
    let review = canonical_review(&entry.morphism).expect("validated canonical review");
    if review.target_id.as_str() != input.claim_cell_id {
        return Err(vec![finding(
            "verifier_review_target_mismatch",
            ClaimLevel::LedgerVerifiable,
            Some(review.target_id.to_string()),
            "canonical verifier review must target the exact claim bound by the lineage proof",
        )]);
    }
    let latest = latest_evidence_review_entries(input.case_space)
        .get(input.claim_cell_id)
        .copied();
    if latest.map(|entry| &entry.morphism_id) != Some(&entry.morphism_id) {
        return Err(vec![finding(
            "verifier_review_not_current",
            ClaimLevel::LedgerVerifiable,
            Some(entry.morphism_id.to_string()),
            "verifier proof requires the exact latest canonical evidence review entry",
        )]);
    }
    Ok(LedgerDerivedVerifierProof {
        binding,
        verifier_report_id: report.report_id,
        disposition: match review.action {
            ReviewAction::Accept => VerifierDisposition::Accept,
            ReviewAction::Reject => VerifierDisposition::Reject,
            _ => VerifierDisposition::Abstain,
        },
        runtime_attestations: Vec::new(),
    })
}

fn current_lineage_findings(
    case_space: &CaseSpace,
    binding: &LedgerLineageBinding,
    expected_operation: &str,
    expected_morphism: &str,
    verifier_disposition: Option<VerifierDisposition>,
) -> Vec<PolicyFinding> {
    let mut findings = Vec::new();
    if evaluate_native_case(case_space).is_err()
        || case_space.case_space_id.as_str() != binding.case_space_id
    {
        findings.push(finding(
            "lineage_current_case_mismatch",
            ClaimLevel::LedgerVerifiable,
            Some(binding.authority_morphism_id.clone()),
            "policy reconciliation requires the current canonical case space",
        ));
        return findings;
    }
    if check_operation_gate(case_space, &binding.operation_gate, expected_operation).is_err() {
        findings.push(finding(
            "lineage_current_operation_gate_invalid",
            ClaimLevel::LedgerVerifiable,
            Some(binding.actor_id.clone()),
            "the proof's actor/capability gate is no longer valid in the current ledger",
        ));
    }
    let claim_matches = case_space.case_cells.iter().any(|cell| {
        cell.id.as_str() == binding.claim_cell_id
            && cell
                .metadata
                .get("execution_topology_content_hash")
                .and_then(serde_json::Value::as_str)
                == Some(binding.topology_content_hash.as_str())
    });
    if !claim_matches {
        findings.push(finding(
            "lineage_current_claim_mismatch",
            ClaimLevel::LedgerVerifiable,
            Some(binding.claim_cell_id.clone()),
            "the proof's claim/topology binding is not present in the current ledger",
        ));
    }
    let authority = case_space
        .morphism_log
        .iter()
        .find(|entry| entry.morphism_id.as_str() == binding.authority_morphism_id);
    if !authority
        .is_some_and(|entry| lineage_authority_matches(entry, binding, expected_morphism, None))
    {
        findings.push(finding(
            "lineage_current_authority_invalid",
            ClaimLevel::LedgerVerifiable,
            Some(binding.authority_morphism_id.clone()),
            "the proof's canonical authority morphism is absent or no longer matches",
        ));
    }
    if let Some(disposition) = verifier_disposition {
        let expected = match disposition {
            VerifierDisposition::Accept => Some(ReviewStatus::Accepted),
            VerifierDisposition::Reject => Some(ReviewStatus::Rejected),
            VerifierDisposition::Abstain => None,
        };
        if latest_evidence_review_status(case_space, &binding.claim_cell_id) != expected {
            findings.push(finding(
                "verifier_review_no_longer_effective",
                ClaimLevel::LedgerVerifiable,
                Some(binding.authority_morphism_id.clone()),
                "the current log-derived review status no longer supports this verifier disposition",
            ));
        }
    }
    findings
}

pub fn reconcile_verification_policy(
    case_space: &CaseSpace,
    policy: &VerificationPolicy,
    producer: &LedgerDerivedProducerProof,
    verifiers: &[LedgerDerivedVerifierProof],
    anchors: &[ToolObservedAnchorProof],
) -> VerificationPolicyResult {
    let mut current_findings = current_lineage_findings(
        case_space,
        &producer.binding,
        "dispatch",
        "execution_trace_anchor",
        None,
    );
    for verifier in verifiers {
        current_findings.extend(current_lineage_findings(
            case_space,
            &verifier.binding,
            "review",
            "review",
            Some(verifier.disposition),
        ));
    }
    let mut result = reconcile_bound_verification_policy(policy, producer, verifiers, anchors);
    if !current_findings.is_empty() {
        result.findings.append(&mut current_findings);
        result.findings.sort_by(|left, right| {
            (&left.code, &left.subject_id).cmp(&(&right.code, &right.subject_id))
        });
        result.ledger_requirements_satisfied = false;
        result.quorum_satisfied = false;
        result.policy_satisfied = false;
    }
    result
}

fn reconcile_bound_verification_policy(
    policy: &VerificationPolicy,
    producer: &LedgerDerivedProducerProof,
    verifiers: &[LedgerDerivedVerifierProof],
    anchors: &[ToolObservedAnchorProof],
) -> VerificationPolicyResult {
    let mut findings = validate_verification_policy(policy);
    let producer_caps = producer
        .binding
        .capability_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let required_producer = policy
        .producer_constraints
        .capability_ids
        .iter()
        .map(String::as_str);
    if !required_producer
        .into_iter()
        .all(|id| producer_caps.contains(id))
    {
        findings.push(finding(
            "producer_capability_missing",
            ClaimLevel::LedgerVerifiable,
            Some(producer.binding.actor_id.clone()),
            "producer lineage lacks a required capability",
        ));
    }

    let required_verifier_caps = policy
        .verifier_constraints
        .capability_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut qualifying_accepts = 0_u32;
    let mut qualifying_verifiers = 0_u32;
    let mut runtime_ok = true;
    let mut seen_verifier_reports = BTreeSet::new();
    let mut seen_verifier_actors = BTreeSet::new();
    for verifier in verifiers {
        if verifier.verifier_report_id.is_empty()
            || !seen_verifier_reports.insert(verifier.verifier_report_id.as_str())
        {
            findings.push(finding(
                "duplicate_or_empty_verifier_report",
                ClaimLevel::LedgerVerifiable,
                Some(verifier.verifier_report_id.clone()),
                "each quorum member must have a unique non-empty verifier report id",
            ));
            continue;
        }
        if verifier.binding.actor_id.is_empty()
            || !seen_verifier_actors.insert(verifier.binding.actor_id.as_str())
        {
            findings.push(finding(
                "duplicate_or_empty_verifier_actor",
                ClaimLevel::LedgerVerifiable,
                Some(verifier.verifier_report_id.clone()),
                "each quorum member must have a unique non-empty ledger actor id",
            ));
            continue;
        }
        let capabilities = verifier
            .binding
            .capability_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let has_capabilities = required_verifier_caps.is_subset(&capabilities);
        let same_binding = verifier.binding.case_space_id == producer.binding.case_space_id
            && verifier.binding.case_revision_id == producer.binding.case_revision_id
            && verifier.binding.claim_cell_id == producer.binding.claim_cell_id
            && verifier.binding.topology_content_hash == producer.binding.topology_content_hash
            && verifier.binding.node_id == producer.binding.node_id
            && verifier.binding.attempt_id == producer.binding.attempt_id;
        if !same_binding {
            findings.push(finding(
                "verifier_lineage_binding_mismatch",
                ClaimLevel::LedgerVerifiable,
                Some(verifier.verifier_report_id.clone()),
                "producer and verifier proofs must bind the same exact revision, claim, topology, node, and attempt",
            ));
        }
        let differs =
            !policy.actor_must_differ || verifier.binding.actor_id != producer.binding.actor_id;
        if policy.actor_must_differ && !differs {
            findings.push(finding(
                "same_actor_policy_violation",
                ClaimLevel::LedgerVerifiable,
                Some(verifier.verifier_report_id.clone()),
                "configured actor_must_differ constraint was not met",
            ));
        }
        let declared = verifier
            .runtime_attestations
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let attestations_allowed = declared.iter().all(|attestation| {
            policy
                .allowed_runtime_attestations
                .iter()
                .any(|allowed| allowed == *attestation)
        });
        if !attestations_allowed {
            runtime_ok = false;
            findings.push(finding(
                "runtime_attestation_not_allowed",
                ClaimLevel::RuntimeAttested,
                Some(verifier.verifier_report_id.clone()),
                "runtime supplied an attestation the policy does not allow",
            ));
        }
        if has_capabilities && differs && same_binding {
            qualifying_verifiers += 1;
            if verifier.disposition == VerifierDisposition::Accept {
                qualifying_accepts += 1;
            }
        }
    }
    let quorum_satisfied = qualifying_verifiers == policy.quorum.total_verifiers
        && qualifying_accepts >= policy.quorum.minimum_accepts;
    if !quorum_satisfied {
        findings.push(finding(
            "quorum_not_satisfied",
            ClaimLevel::LedgerVerifiable,
            None,
            format!(
                "required {} accepts from exactly {} qualifying verifiers; observed {qualifying_accepts} accepts from {qualifying_verifiers}",
                policy.quorum.minimum_accepts, policy.quorum.total_verifiers
            ),
        ));
    }

    let mut anchors_by_id = std::collections::BTreeMap::new();
    let mut anchor_identity_valid = true;
    for anchor in anchors {
        if anchor.id().is_empty() || anchors_by_id.insert(anchor.id(), anchor).is_some() {
            anchor_identity_valid = false;
            findings.push(finding(
                "duplicate_or_empty_anchor_id",
                ClaimLevel::LedgerVerifiable,
                Some(anchor.id().to_owned()),
                "world anchor ids must be unique and non-empty",
            ));
        }
    }
    let anchors_satisfied = policy.required_anchors.iter().all(|required| {
        anchors_by_id
            .get(required.as_str())
            .is_some_and(|proof| proof.provenance_is_well_formed())
    }) && anchor_identity_valid;
    if !anchors_satisfied {
        findings.push(finding(
            "required_anchor_not_satisfied",
            ClaimLevel::LedgerVerifiable,
            None,
            "one or more required world anchors are absent or failed deterministic validation",
        ));
    }
    findings.push(finding(
        "independent_minds_not_observable",
        ClaimLevel::NotObservableHere,
        None,
        "different actor ids do not prove independent minds or undeclared information isolation",
    ));
    findings.push(finding(
        "fresh_context_not_observable",
        ClaimLevel::NotObservableHere,
        None,
        "runtime context metadata cannot prove genuine context freshness",
    ));
    findings.sort_by(|left, right| {
        (&left.code, &left.subject_id).cmp(&(&right.code, &right.subject_id))
    });
    let ledger_requirements_satisfied = !findings.iter().any(|finding| {
        finding.level == ClaimLevel::LedgerVerifiable
            && !matches!(
                finding.code.as_str(),
                "independent_minds_not_observable" | "fresh_context_not_observable"
            )
    });
    let policy_satisfied =
        ledger_requirements_satisfied && runtime_ok && anchors_satisfied && quorum_satisfied;
    VerificationPolicyResult {
        policy_id: policy.verification_policy_id.clone(),
        ledger_scope: LedgerDerivedLineageScope {
            case_space_id: producer.binding.case_space_id.clone(),
            case_revision_id: producer.binding.case_revision_id.clone(),
            claim_cell_id: producer.binding.claim_cell_id.clone(),
            topology_content_hash: producer.binding.topology_content_hash.clone(),
            node_id: producer.binding.node_id.clone(),
            attempt_id: producer.binding.attempt_id.clone(),
        },
        ledger_requirements_satisfied,
        runtime_attestations_satisfied: runtime_ok,
        anchors_satisfied,
        quorum_satisfied,
        policy_satisfied,
        independent_minds_proven: false,
        fresh_context_proven: false,
        findings,
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn finding(
    code: &str,
    level: ClaimLevel,
    subject_id: Option<String>,
    detail: impl Into<String>,
) -> PolicyFinding {
    PolicyFinding {
        code: code.to_owned(),
        level,
        subject_id,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE_BYTES: &[u8] = b"deterministic reference anchor\n";

    fn policy() -> VerificationPolicy {
        parse_verification_policy(include_str!(
            "../schemas/experimental/verification.policy.example.json"
        ))
        .unwrap()
    }

    fn proof_binding(actor: &str, capabilities: Vec<String>) -> LedgerLineageBinding {
        LedgerLineageBinding {
            case_space_id: "case_space:test".into(),
            observed_case_revision_id: "revision:observed".into(),
            case_revision_id: "revision:test".into(),
            claim_cell_id: "claim:test".into(),
            topology_content_hash: "a".repeat(64),
            node_id: "node:test".into(),
            attempt_id: "attempt:test:1".into(),
            actor_id: actor.into(),
            capability_ids: capabilities,
            operation_gate: NativeOperationGate {
                actor_id: higher_graphen_core::Id::new(actor).unwrap(),
                operation: "test".into(),
                operation_scope_id: higher_graphen_core::Id::new("case_space:test").unwrap(),
                audience: crate::native_model::ProjectionAudience::Audit,
                capability_ids: vec![],
                source_boundary_id: higher_graphen_core::Id::new("source_boundary:test").unwrap(),
            },
            operation_gate_content_hash: "b".repeat(64),
            runtime_report_content_hash: "c".repeat(64),
            execution_trace_content_hash: "d".repeat(64),
            authority_morphism_id: "morphism:test".into(),
        }
    }

    fn producer() -> LedgerDerivedProducerProof {
        LedgerDerivedProducerProof {
            binding: proof_binding("actor:producer", vec!["capability:research".into()]),
        }
    }

    fn verifier(
        id: &str,
        actor: &str,
        disposition: VerifierDisposition,
    ) -> LedgerDerivedVerifierProof {
        LedgerDerivedVerifierProof {
            binding: proof_binding(actor, vec!["capability:review".into()]),
            verifier_report_id: id.into(),
            disposition,
            runtime_attestations: vec!["separate_session".into()],
        }
    }

    fn anchor() -> ToolObservedAnchorProof {
        let artifact_id = format!("artifact:sha256-{}", sha256_hex(REFERENCE_BYTES));
        observe_trusted_reference_artifact(
            &TrustedReferenceAnchorAdapter::new("adapter:deterministic-test"),
            "anchor:source",
            &artifact_id,
            REFERENCE_BYTES,
        )
        .expect("real deterministic reference anchor")
    }

    #[test]
    fn example_is_valid_and_quorum_plus_anchor_reconcile() {
        let result = reconcile_bound_verification_policy(
            &policy(),
            &producer(),
            &[
                verifier("review:1", "actor:v1", VerifierDisposition::Accept),
                verifier("review:2", "actor:v2", VerifierDisposition::Accept),
                verifier("review:3", "actor:v3", VerifierDisposition::Reject),
            ],
            &[anchor()],
        );
        assert!(result.policy_satisfied);
        assert!(!result.independent_minds_proven);
        assert!(!result.fresh_context_proven);
    }

    #[test]
    fn same_actor_violates_policy_without_redefining_core_review() {
        let result = reconcile_bound_verification_policy(
            &policy(),
            &producer(),
            &[
                verifier("review:1", "actor:producer", VerifierDisposition::Accept),
                verifier("review:2", "actor:v2", VerifierDisposition::Accept),
                verifier("review:3", "actor:v3", VerifierDisposition::Accept),
            ],
            &[anchor()],
        );
        assert!(!result.policy_satisfied);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.code == "same_actor_policy_violation"));
    }

    #[test]
    fn runtime_metadata_never_proves_freshness_or_independence() {
        let mut records = vec![
            verifier("review:1", "actor:v1", VerifierDisposition::Accept),
            verifier("review:2", "actor:v2", VerifierDisposition::Accept),
            verifier("review:3", "actor:v3", VerifierDisposition::Reject),
        ];
        records[0].runtime_attestations = vec!["separate_session".into()];
        let result =
            reconcile_bound_verification_policy(&policy(), &producer(), &records, &[anchor()]);
        assert!(result.policy_satisfied);
        assert!(!result.independent_minds_proven && !result.fresh_context_proven);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.level == ClaimLevel::NotObservableHere));
    }

    #[test]
    fn caller_declared_lineage_never_satisfies_ledger_requirements() {
        let declared = reconcile_declared_lineage(
            &policy(),
            &DeclaredProducerLineage {
                actor_id: "actor:producer".into(),
                capability_ids: vec!["capability:research".into()],
            },
            &[DeclaredVerifierRecord {
                verifier_report_id: "review:declared".into(),
                actor_id: "actor:verifier".into(),
                capability_ids: vec!["capability:review".into()],
                disposition: VerifierDisposition::Accept,
                runtime_attestations: vec!["separate_session".into()],
            }],
        );
        assert!(declared.declarations_well_formed);
        assert!(!declared.ledger_requirements_satisfied);
        assert!(declared.findings.iter().any(|finding| {
            finding.code == "declared_lineage_not_ledger_authority"
                && finding.level == ClaimLevel::RuntimeAttested
        }));
    }

    #[test]
    fn cross_revision_claim_or_attempt_proofs_fail_closed() {
        let producer = producer();
        for mutate in ["revision", "claim", "attempt"] {
            let mut record = verifier("review:1", "actor:v1", VerifierDisposition::Accept);
            match mutate {
                "revision" => record.binding.case_revision_id = "revision:other".into(),
                "claim" => record.binding.claim_cell_id = "claim:other".into(),
                "attempt" => record.binding.attempt_id = "attempt:other:1".into(),
                _ => unreachable!(),
            }
            let result = reconcile_bound_verification_policy(
                &policy(),
                &producer,
                &[
                    record,
                    verifier("review:2", "actor:v2", VerifierDisposition::Accept),
                    verifier("review:3", "actor:v3", VerifierDisposition::Accept),
                ],
                &[anchor()],
            );
            assert!(!result.policy_satisfied, "{mutate} substitution must fail");
            assert!(result
                .findings
                .iter()
                .any(|finding| finding.code == "verifier_lineage_binding_mismatch"));
        }
    }

    fn producer_derivation_fixture() -> (
        CaseSpace,
        NativeOperationGate,
        Vec<u8>,
        Vec<u8>,
        String,
        String,
        String,
        String,
    ) {
        let mut case_space: CaseSpace = serde_json::from_str(include_str!(
            "../schemas/casegraphen/native.case.space.example.json"
        ))
        .unwrap();
        let topology_hash = "1".repeat(64);
        let node_id = "node:review-file-a".to_owned();
        let attempt_id = "attempt:review-file-a:1".to_owned();
        let claim_cell_id = case_space
            .case_cells
            .iter()
            .find(|cell| cell.cell_type == CaseCellType::Work)
            .unwrap()
            .id
            .to_string();
        case_space
            .case_cells
            .iter_mut()
            .find(|cell| cell.id.as_str() == claim_cell_id)
            .unwrap()
            .metadata
            .insert(
                "execution_topology_content_hash".into(),
                serde_json::json!(topology_hash),
            );
        case_space.morphism_log[0].morphism.metadata["payload"]["added_cells"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|cell| cell["id"].as_str() == Some(claim_cell_id.as_str()))
            .unwrap()["metadata"]["execution_topology_content_hash"] =
            serde_json::json!(topology_hash);
        let gate = NativeOperationGate {
            actor_id: higher_graphen_core::Id::new("actor:native-run").unwrap(),
            operation: "dispatch".into(),
            operation_scope_id: case_space.case_space_id.clone(),
            audience: crate::native_model::ProjectionAudience::Audit,
            capability_ids: vec![higher_graphen_core::Id::new("capability:dispatch").unwrap()],
            source_boundary_id: higher_graphen_core::Id::new(
                "source_boundary:native-case-management-contract",
            )
            .unwrap(),
        };
        let mut report: RuntimeNodeReport = serde_json::from_str(include_str!(
            "../schemas/experimental/runtime.node_report.example.json"
        ))
        .unwrap();
        report.runtime_graph_content_hash = topology_hash.clone();
        report.node_id = node_id.clone();
        report.attempt_id = attempt_id.clone();
        let report_bytes = serde_json::to_vec(&report).unwrap();
        let authority_morphism_id = "morphism:runtime-dispatch-lineage".to_owned();
        let previous_revision = case_space.revision.revision_id.clone();
        let current_revision =
            higher_graphen_core::Id::new("revision:runtime-dispatch-lineage").unwrap();
        let authority_entry_id =
            higher_graphen_core::Id::new("morphism_log_entry:runtime-dispatch-lineage").unwrap();
        let mut trace: ExecutionTrace = serde_json::from_str(include_str!(
            "../schemas/casegraphen/execution.trace.example.json"
        ))
        .unwrap();
        trace.case_space_id = case_space.case_space_id.clone();
        trace.operation_gate = gate.clone();
        trace.result_revision_id = Some(current_revision.clone());
        trace.appended_entry_ids = vec![authority_entry_id.clone()];
        let trace_bytes = serde_json::to_vec(&trace).unwrap();
        let gate_hash = sha256_hex(&serde_json::to_vec(&gate).unwrap());
        let mut entry = case_space.morphism_log.last().unwrap().clone();
        entry.sequence += 1;
        entry.entry_id = authority_entry_id;
        entry.morphism_id = higher_graphen_core::Id::new(&authority_morphism_id).unwrap();
        entry.morphism.morphism_id = entry.morphism_id.clone();
        entry.actor_id = gate.actor_id.clone();
        entry.source_revision_id = Some(previous_revision.clone());
        entry.target_revision_id = current_revision.clone();
        entry.morphism.source_revision_id = Some(previous_revision.clone());
        entry.morphism.target_revision_id = current_revision.clone();
        entry.morphism.morphism_type = CaseMorphismType::Custom("execution_trace_anchor".into());
        entry.morphism.added_ids.clear();
        entry.morphism.updated_ids.clear();
        entry.morphism.retired_ids.clear();
        entry.morphism.preserved_ids.clear();
        entry.morphism.evidence_ids.clear();
        entry.morphism.review_status = ReviewStatus::Accepted;
        entry.morphism.metadata = serde_json::Map::from_iter([
            (
                "case_revision_id".into(),
                serde_json::json!(trace.base_revision_id),
            ),
            ("claim_cell_id".into(), serde_json::json!(claim_cell_id)),
            (
                "topology_content_hash".into(),
                serde_json::json!(topology_hash),
            ),
            ("node_id".into(), serde_json::json!(node_id)),
            ("attempt_id".into(), serde_json::json!(attempt_id)),
            (
                "operation_gate_content_hash".into(),
                serde_json::json!(gate_hash),
            ),
            (
                "runtime_report_content_hash".into(),
                serde_json::json!(sha256_hex(&report_bytes)),
            ),
            (
                "execution_trace_content_hash".into(),
                serde_json::json!(sha256_hex(&trace_bytes)),
            ),
            ("trace_id".into(), serde_json::json!(trace.trace_id)),
            (
                "trace_content_hash".into(),
                serde_json::json!(sha256_hex(&trace_bytes)),
            ),
        ]);
        case_space.morphism_log.push(entry.clone());
        case_space.revision.revision_id = entry.target_revision_id;
        case_space.revision.parent_revision_id = Some(previous_revision);
        case_space.revision.applied_entry_ids = vec![entry.entry_id];
        case_space.revision.applied_morphism_ids = vec![entry.morphism_id];
        case_space.revision.checksum = entry.replay_checksum;
        (
            case_space,
            gate,
            report_bytes,
            trace_bytes,
            claim_cell_id,
            topology_hash,
            node_id,
            authority_morphism_id,
        )
    }

    #[test]
    fn producer_proof_derives_only_from_exact_ledger_and_bytes() {
        let (case_space, gate, report, trace, claim, topology, node, morphism) =
            producer_derivation_fixture();
        let proof = derive_ledger_producer_proof(LedgerLineageDerivation {
            case_space: &case_space,
            claim_cell_id: &claim,
            topology_content_hash: &topology,
            node_id: &node,
            attempt_id: "attempt:review-file-a:1",
            operation_gate: &gate,
            authority_morphism_id: &morphism,
            runtime_report_bytes: &report,
            execution_trace_bytes: &trace,
        })
        .expect("exact ledger lineage should derive");
        let parsed_trace: ExecutionTrace = serde_json::from_slice(&trace).unwrap();
        assert_eq!(
            proof.binding.case_revision_id,
            parsed_trace.base_revision_id.as_str()
        );
        assert_eq!(
            proof.binding.observed_case_revision_id,
            case_space.revision.revision_id.as_str()
        );

        let mut substituted = report;
        substituted.push(b' ');
        let error = derive_ledger_producer_proof(LedgerLineageDerivation {
            case_space: &case_space,
            claim_cell_id: &claim,
            topology_content_hash: &topology,
            node_id: &node,
            attempt_id: "attempt:review-file-a:1",
            operation_gate: &gate,
            authority_morphism_id: &morphism,
            runtime_report_bytes: &substituted,
            execution_trace_bytes: &trace,
        })
        .expect_err("substituted report bytes must fail closed");
        assert!(error
            .iter()
            .any(|finding| finding.code == "lineage_authority_morphism_mismatch"));
    }

    #[test]
    fn retired_capability_and_forged_actor_cannot_derive_producer_proof() {
        for mutation in ["retired_capability", "forged_actor"] {
            let (mut case_space, mut gate, report, trace, claim, topology, node, morphism) =
                producer_derivation_fixture();
            if mutation == "retired_capability" {
                case_space
                    .case_cells
                    .iter_mut()
                    .find(|cell| cell.id == gate.capability_ids[0])
                    .unwrap()
                    .lifecycle = CaseCellLifecycle::Retired;
            } else {
                gate.actor_id = higher_graphen_core::Id::new("actor:forged").unwrap();
            }
            let error = derive_ledger_producer_proof(LedgerLineageDerivation {
                case_space: &case_space,
                claim_cell_id: &claim,
                topology_content_hash: &topology,
                node_id: &node,
                attempt_id: "attempt:review-file-a:1",
                operation_gate: &gate,
                authority_morphism_id: &morphism,
                runtime_report_bytes: &report,
                execution_trace_bytes: &trace,
            })
            .expect_err("non-canonical authority must fail closed");
            assert!(error
                .iter()
                .any(|finding| finding.code == "lineage_operation_gate_invalid"));
        }
    }

    #[test]
    fn verifier_proof_requires_canonical_review_target_and_exact_bytes() {
        let (mut case_space, mut gate, report, trace, claim, topology, node, morphism) =
            producer_derivation_fixture();
        let producer = derive_ledger_producer_proof(LedgerLineageDerivation {
            case_space: &case_space,
            claim_cell_id: &claim,
            topology_content_hash: &topology,
            node_id: &node,
            attempt_id: "attempt:review-file-a:1",
            operation_gate: &gate,
            authority_morphism_id: &morphism,
            runtime_report_bytes: &report,
            execution_trace_bytes: &trace,
        })
        .unwrap();
        gate.actor_id = higher_graphen_core::Id::new("actor:native-mutation-cli").unwrap();
        gate.operation = "review".into();
        gate.capability_ids =
            vec![higher_graphen_core::Id::new("capability:durable-mutation").unwrap()];
        let mut parsed_trace: ExecutionTrace = serde_json::from_slice(&trace).unwrap();
        let authority_entry = case_space.morphism_log.last().unwrap();
        parsed_trace.base_revision_id = authority_entry.source_revision_id.clone().unwrap();
        parsed_trace.result_revision_id = Some(authority_entry.target_revision_id.clone());
        parsed_trace.appended_entry_ids = vec![authority_entry.entry_id.clone()];
        parsed_trace.operation_gate = gate.clone();
        let trace = serde_json::to_vec(&parsed_trace).unwrap();
        let gate_hash = sha256_hex(&serde_json::to_vec(&gate).unwrap());
        let entry = case_space.morphism_log.last_mut().unwrap();
        entry.actor_id = gate.actor_id.clone();
        entry.morphism.morphism_type = CaseMorphismType::Review;
        entry.morphism.metadata.insert(
            "operation_gate_content_hash".into(),
            serde_json::json!(gate_hash),
        );
        entry.morphism.metadata.insert(
            "execution_trace_content_hash".into(),
            serde_json::json!(sha256_hex(&trace)),
        );
        entry.morphism.metadata.extend([
            ("native_review_schema_version".into(), serde_json::json!(1)),
            ("review_id".into(), serde_json::json!("review:lineage")),
            ("target_kind".into(), serde_json::json!("evidence")),
            ("target_id".into(), serde_json::json!(claim)),
            ("action".into(), serde_json::json!("accept")),
            (
                "outcome_review_status".into(),
                serde_json::json!("accepted"),
            ),
            (
                "reviewer_id".into(),
                serde_json::json!("actor:native-mutation-cli"),
            ),
            (
                "reviewed_at".into(),
                serde_json::json!("2026-08-04T00:00:00Z"),
            ),
            ("reason".into(), serde_json::json!("lineage review")),
        ]);
        let proof = derive_ledger_verifier_proof(
            LedgerLineageDerivation {
                case_space: &case_space,
                claim_cell_id: &claim,
                topology_content_hash: &topology,
                node_id: &node,
                attempt_id: "attempt:review-file-a:1",
                operation_gate: &gate,
                authority_morphism_id: &morphism,
                runtime_report_bytes: &report,
                execution_trace_bytes: &trace,
            },
            &producer,
        )
        .expect("canonical exact review should derive verifier proof");
        assert_eq!(proof.binding.claim_cell_id, claim);

        case_space
            .morphism_log
            .last_mut()
            .unwrap()
            .morphism
            .metadata
            .insert("action".into(), serde_json::json!("reject"));
        case_space
            .morphism_log
            .last_mut()
            .unwrap()
            .morphism
            .metadata
            .insert(
                "outcome_review_status".into(),
                serde_json::json!("rejected"),
            );
        let rejected = derive_ledger_verifier_proof(
            LedgerLineageDerivation {
                case_space: &case_space,
                claim_cell_id: &claim,
                topology_content_hash: &topology,
                node_id: &node,
                attempt_id: "attempt:review-file-a:1",
                operation_gate: &gate,
                authority_morphism_id: &morphism,
                runtime_report_bytes: &report,
                execution_trace_bytes: &trace,
            },
            &producer,
        )
        .expect("canonical rejection should derive a rejected disposition");
        assert_eq!(rejected.disposition, VerifierDisposition::Reject);

        case_space
            .morphism_log
            .last_mut()
            .unwrap()
            .morphism
            .metadata
            .insert("target_id".into(), serde_json::json!("claim:other"));
        let error = derive_ledger_verifier_proof(
            LedgerLineageDerivation {
                case_space: &case_space,
                claim_cell_id: &claim,
                topology_content_hash: &topology,
                node_id: &node,
                attempt_id: "attempt:review-file-a:1",
                operation_gate: &gate,
                authority_morphism_id: &morphism,
                runtime_report_bytes: &report,
                execution_trace_bytes: &trace,
            },
            &producer,
        )
        .expect_err("reviewing another target cannot derive this claim proof");
        assert!(error
            .iter()
            .any(|finding| finding.code == "verifier_review_target_mismatch"));
    }

    #[test]
    fn producer_and_later_verifier_proofs_compose_on_the_shared_subject_revision() {
        let (
            mut case_space,
            producer_gate,
            report,
            producer_trace,
            claim,
            topology,
            node,
            producer_morphism,
        ) = producer_derivation_fixture();
        let producer_revision = case_space.revision.revision_id.clone();
        let producer_entry_hash =
            crate::native_hash::morphism_log_entry_hash(case_space.morphism_log.last().unwrap())
                .unwrap();

        let mut verifier_gate = producer_gate.clone();
        verifier_gate.actor_id = higher_graphen_core::Id::new("actor:native-mutation-cli").unwrap();
        verifier_gate.operation = "review".into();
        verifier_gate.capability_ids =
            vec![higher_graphen_core::Id::new("capability:durable-mutation").unwrap()];
        let verifier_revision = higher_graphen_core::Id::new("revision:lineage-review").unwrap();
        let verifier_entry_id =
            higher_graphen_core::Id::new("morphism_log_entry:lineage-review").unwrap();
        let verifier_morphism = "morphism:lineage-review";
        let mut verifier_trace: ExecutionTrace = serde_json::from_slice(&producer_trace).unwrap();
        let subject_revision = verifier_trace.base_revision_id.clone();
        verifier_trace.base_revision_id = producer_revision.clone();
        verifier_trace.operation_gate = verifier_gate.clone();
        verifier_trace.result_revision_id = Some(verifier_revision.clone());
        verifier_trace.appended_entry_ids = vec![verifier_entry_id.clone()];
        let verifier_trace_bytes = serde_json::to_vec(&verifier_trace).unwrap();
        let mut verifier_entry = case_space.morphism_log.last().unwrap().clone();
        verifier_entry.sequence += 1;
        verifier_entry.entry_id = verifier_entry_id;
        verifier_entry.morphism_id = higher_graphen_core::Id::new(verifier_morphism).unwrap();
        verifier_entry.morphism.morphism_id = verifier_entry.morphism_id.clone();
        verifier_entry.source_revision_id = Some(producer_revision.clone());
        verifier_entry.target_revision_id = verifier_revision.clone();
        verifier_entry.morphism.source_revision_id = Some(producer_revision.clone());
        verifier_entry.morphism.target_revision_id = verifier_revision.clone();
        verifier_entry.actor_id = verifier_gate.actor_id.clone();
        verifier_entry.previous_entry_hash = Some(producer_entry_hash);
        verifier_entry.morphism.morphism_type = CaseMorphismType::Review;
        verifier_entry.morphism.review_status = ReviewStatus::Accepted;
        verifier_entry.morphism.metadata = serde_json::Map::from_iter([
            (
                "case_revision_id".into(),
                serde_json::json!(subject_revision),
            ),
            ("claim_cell_id".into(), serde_json::json!(claim)),
            ("topology_content_hash".into(), serde_json::json!(topology)),
            ("node_id".into(), serde_json::json!(node)),
            (
                "attempt_id".into(),
                serde_json::json!("attempt:review-file-a:1"),
            ),
            (
                "operation_gate_content_hash".into(),
                serde_json::json!(sha256_hex(&serde_json::to_vec(&verifier_gate).unwrap())),
            ),
            (
                "runtime_report_content_hash".into(),
                serde_json::json!(sha256_hex(&report)),
            ),
            (
                "execution_trace_content_hash".into(),
                serde_json::json!(sha256_hex(&verifier_trace_bytes)),
            ),
            ("native_review_schema_version".into(), serde_json::json!(1)),
            (
                "review_id".into(),
                serde_json::json!("review:lineage-composed"),
            ),
            ("target_kind".into(), serde_json::json!("evidence")),
            ("target_id".into(), serde_json::json!(claim)),
            ("action".into(), serde_json::json!("accept")),
            (
                "outcome_review_status".into(),
                serde_json::json!("accepted"),
            ),
            (
                "reviewer_id".into(),
                serde_json::json!(verifier_gate.actor_id),
            ),
            (
                "reviewed_at".into(),
                serde_json::json!("2026-08-04T00:00:00Z"),
            ),
            (
                "reason".into(),
                serde_json::json!("independent lineage review"),
            ),
        ]);
        case_space.morphism_log.push(verifier_entry.clone());
        case_space.revision.revision_id = verifier_revision;
        case_space.revision.parent_revision_id = Some(producer_revision);
        case_space.revision.applied_entry_ids = vec![verifier_entry.entry_id.clone()];
        case_space.revision.applied_morphism_ids = vec![verifier_entry.morphism_id.clone()];
        case_space.revision.checksum = verifier_entry.replay_checksum;

        let producer = derive_ledger_producer_proof(LedgerLineageDerivation {
            case_space: &case_space,
            claim_cell_id: &claim,
            topology_content_hash: &topology,
            node_id: &node,
            attempt_id: "attempt:review-file-a:1",
            operation_gate: &producer_gate,
            authority_morphism_id: &producer_morphism,
            runtime_report_bytes: &report,
            execution_trace_bytes: &producer_trace,
        })
        .expect("historical producer authority remains derivable from the current ledger");
        let verifier = derive_ledger_verifier_proof(
            LedgerLineageDerivation {
                case_space: &case_space,
                claim_cell_id: &claim,
                topology_content_hash: &topology,
                node_id: &node,
                attempt_id: "attempt:review-file-a:1",
                operation_gate: &verifier_gate,
                authority_morphism_id: verifier_morphism,
                runtime_report_bytes: &report,
                execution_trace_bytes: &verifier_trace_bytes,
            },
            &producer,
        )
        .expect("later verifier authority derives from the same deployment subject");
        assert_ne!(
            producer.binding.observed_case_revision_id,
            producer.binding.case_revision_id
        );
        assert_eq!(
            producer.binding.case_revision_id,
            verifier.binding.case_revision_id
        );

        let mut composed_policy = policy();
        composed_policy.producer_constraints.capability_ids =
            producer.binding.capability_ids.clone();
        composed_policy.verifier_constraints.capability_ids =
            verifier.binding.capability_ids.clone();
        composed_policy.quorum = VerificationQuorum {
            minimum_accepts: 1,
            total_verifiers: 1,
        };
        composed_policy.required_anchors.clear();
        let result = reconcile_verification_policy(
            &case_space,
            &composed_policy,
            &producer,
            std::slice::from_ref(&verifier),
            &[],
        );
        assert!(result.policy_satisfied, "{:?}", result.findings);

        let previous_hash =
            crate::native_hash::morphism_log_entry_hash(case_space.morphism_log.last().unwrap())
                .unwrap();
        let mut reopen = case_space.morphism_log.last().unwrap().clone();
        let reopen_revision = higher_graphen_core::Id::new("revision:lineage-reopen").unwrap();
        reopen.sequence += 1;
        reopen.entry_id =
            higher_graphen_core::Id::new("morphism_log_entry:lineage-reopen").unwrap();
        reopen.morphism_id = higher_graphen_core::Id::new("morphism:lineage-reopen").unwrap();
        reopen.morphism.morphism_id = reopen.morphism_id.clone();
        reopen.source_revision_id = Some(case_space.revision.revision_id.clone());
        reopen.target_revision_id = reopen_revision.clone();
        reopen.morphism.source_revision_id = reopen.source_revision_id.clone();
        reopen.morphism.target_revision_id = reopen_revision.clone();
        reopen.previous_entry_hash = Some(previous_hash);
        reopen
            .morphism
            .metadata
            .insert("action".into(), serde_json::json!("reopen"));
        reopen.morphism.metadata.insert(
            "outcome_review_status".into(),
            serde_json::json!("unreviewed"),
        );
        case_space.morphism_log.push(reopen.clone());
        case_space.revision.parent_revision_id = reopen.source_revision_id.clone();
        case_space.revision.revision_id = reopen_revision;
        case_space.revision.applied_entry_ids = vec![reopen.entry_id];
        case_space.revision.applied_morphism_ids = vec![reopen.morphism_id];
        case_space.revision.checksum = reopen.replay_checksum;
        let error = derive_ledger_verifier_proof(
            LedgerLineageDerivation {
                case_space: &case_space,
                claim_cell_id: &claim,
                topology_content_hash: &topology,
                node_id: &node,
                attempt_id: "attempt:review-file-a:1",
                operation_gate: &verifier_gate,
                authority_morphism_id: verifier_morphism,
                runtime_report_bytes: &report,
                execution_trace_bytes: &verifier_trace_bytes,
            },
            &producer,
        )
        .expect_err("a reopened review cannot be reused as current verifier authority");
        assert!(error
            .iter()
            .any(|finding| finding.code == "verifier_review_not_current"));
        let stale = reconcile_verification_policy(
            &case_space,
            &composed_policy,
            &producer,
            &[verifier],
            &[],
        );
        assert!(!stale.policy_satisfied);
        assert!(stale
            .findings
            .iter()
            .any(|finding| finding.code == "verifier_review_no_longer_effective"));
    }

    #[test]
    fn failed_anchor_or_quorum_fails_closed() {
        let result = reconcile_bound_verification_policy(
            &policy(),
            &producer(),
            &[verifier(
                "review:1",
                "actor:v1",
                VerifierDisposition::Accept,
            )],
            &[],
        );
        assert!(!result.policy_satisfied);
        assert!(!result.anchors_satisfied);
        assert!(!result.quorum_satisfied);
    }

    #[test]
    fn duplicate_report_identity_cannot_fill_quorum() {
        let duplicate = verifier("review:same", "actor:v1", VerifierDisposition::Accept);
        let result = reconcile_bound_verification_policy(
            &policy(),
            &producer(),
            &[
                duplicate.clone(),
                duplicate,
                verifier("review:2", "actor:v2", VerifierDisposition::Accept),
            ],
            &[anchor()],
        );
        assert!(!result.policy_satisfied);
        assert!(!result.quorum_satisfied);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate_or_empty_verifier_report"));
    }

    #[test]
    fn duplicate_actor_or_anchor_identity_cannot_fill_policy() {
        let records = [
            verifier("review:1", "actor:same", VerifierDisposition::Accept),
            verifier("review:2", "actor:same", VerifierDisposition::Accept),
            verifier("review:3", "actor:v3", VerifierDisposition::Accept),
        ];
        let anchors = [anchor(), anchor()];
        let result =
            reconcile_bound_verification_policy(&policy(), &producer(), &records, &anchors);
        assert!(!result.policy_satisfied);
        assert!(!result.quorum_satisfied);
        assert!(!result.anchors_satisfied);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate_or_empty_verifier_actor"));
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate_or_empty_anchor_id"));
    }

    #[test]
    fn copied_hashes_remain_declarations_and_cannot_satisfy_observed_policy() {
        let hash = "a".repeat(64);
        let declaration = DeclaredAnchorObservation::SourceArtifactHash {
            anchor_id: "anchor:source".into(),
            expected_sha256: hash.clone(),
            observed_sha256: hash,
        };
        let declared = reconcile_declared_anchors(&policy(), &[declaration]);
        assert!(declared.declared_anchors_match);

        let result = reconcile_bound_verification_policy(
            &policy(),
            &producer(),
            &[
                verifier("review:1", "actor:v1", VerifierDisposition::Accept),
                verifier("review:2", "actor:v2", VerifierDisposition::Accept),
                verifier("review:3", "actor:v3", VerifierDisposition::Reject),
            ],
            &[],
        );
        assert!(!result.anchors_satisfied);
        assert!(!result.policy_satisfied);
    }

    #[test]
    fn trusted_reference_rejects_artifact_substitution() {
        let original_id = format!("artifact:sha256-{}", sha256_hex(REFERENCE_BYTES));
        let error = observe_trusted_reference_artifact(
            &TrustedReferenceAnchorAdapter::new("adapter:deterministic-test"),
            "anchor:source",
            &original_id,
            b"substituted bytes",
        )
        .expect_err("artifact bytes cannot be substituted under an old identity");
        assert!(error
            .iter()
            .any(|finding| finding.code == "trusted_reference_artifact_mismatch"));
    }

    #[test]
    fn missing_case_artifact_cannot_create_an_observed_proof() {
        let case_space: CaseSpace = serde_json::from_str(include_str!(
            "../schemas/casegraphen/native.case.space.example.json"
        ))
        .unwrap();
        let bytes = b"artifact bytes not present in the ledger";
        let artifact_id = format!("artifact:sha256-{}", sha256_hex(bytes));
        let error = observe_case_artifact(&case_space, "anchor:source", &artifact_id, bytes)
            .expect_err("missing ledger artifact must fail closed");
        assert!(error
            .iter()
            .any(|finding| finding.code == "case_artifact_content_mismatch"));
    }

    #[test]
    fn unanchored_or_substituted_trace_bytes_fail_closed() {
        let case_space: CaseSpace = serde_json::from_str(include_str!(
            "../schemas/casegraphen/native.case.space.example.json"
        ))
        .unwrap();
        let error = observe_case_execution_trace(
            &case_space,
            "anchor:source",
            AnchoredExecutionTraceBytes {
                trace: include_bytes!("../schemas/casegraphen/execution.trace.example.json"),
                worker_report: b"copied worker report",
                stdout: b"copied stdout",
                stderr: b"copied stderr",
            },
        )
        .expect_err("a runtime self-report is not a CaseGraphen-anchored trace");
        assert!(error
            .iter()
            .any(|finding| finding.code == "execution_trace_provenance_mismatch"));

        let mut substituted =
            include_bytes!("../schemas/casegraphen/execution.trace.example.json").to_vec();
        substituted.push(b' ');
        assert!(observe_case_execution_trace(
            &case_space,
            "anchor:source",
            AnchoredExecutionTraceBytes {
                trace: &substituted,
                worker_report: b"copied worker report",
                stdout: b"copied stdout",
                stderr: b"copied stderr",
            },
        )
        .is_err());
    }
}

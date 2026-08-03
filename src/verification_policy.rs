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
    native_eval::evaluate_native_case,
    native_hash::sha256_hex,
    native_model::{CaseCellLifecycle, CaseCellType, CaseMorphismType, CaseSpace},
};
use higher_graphen_core::ReviewStatus;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const VERIFICATION_POLICY_SCHEMA: &str = "casegraphen.experimental.verification_policy.v0";

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProducerLineage {
    pub actor_id: String,
    pub capability_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifierRecord {
    pub verifier_report_id: String,
    pub actor_id: String,
    pub capability_ids: Vec<String>,
    pub disposition: VerifierDisposition,
    pub runtime_attestations: Vec<String>,
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
pub struct VerificationPolicyResult {
    pub policy_id: String,
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

pub fn reconcile_verification_policy(
    policy: &VerificationPolicy,
    producer: &ProducerLineage,
    verifiers: &[VerifierRecord],
    anchors: &[ToolObservedAnchorProof],
) -> VerificationPolicyResult {
    let mut findings = validate_verification_policy(policy);
    let producer_caps = producer
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
            Some(producer.actor_id.clone()),
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
        if verifier.actor_id.is_empty() || !seen_verifier_actors.insert(verifier.actor_id.as_str())
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
            .capability_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let has_capabilities = required_verifier_caps.is_subset(&capabilities);
        let differs = !policy.actor_must_differ || verifier.actor_id != producer.actor_id;
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
        if has_capabilities && differs {
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

    fn producer() -> ProducerLineage {
        ProducerLineage {
            actor_id: "actor:producer".into(),
            capability_ids: vec!["capability:research".into()],
        }
    }

    fn verifier(id: &str, actor: &str, disposition: VerifierDisposition) -> VerifierRecord {
        VerifierRecord {
            verifier_report_id: id.into(),
            actor_id: actor.into(),
            capability_ids: vec!["capability:review".into()],
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
        let result = reconcile_verification_policy(
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
        let result = reconcile_verification_policy(
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
        let result = reconcile_verification_policy(&policy(), &producer(), &records, &[anchor()]);
        assert!(result.policy_satisfied);
        assert!(!result.independent_minds_proven && !result.fresh_context_proven);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.level == ClaimLevel::NotObservableHere));
    }

    #[test]
    fn failed_anchor_or_quorum_fails_closed() {
        let result = reconcile_verification_policy(
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
        let result = reconcile_verification_policy(
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
        let result = reconcile_verification_policy(&policy(), &producer(), &records, &anchors);
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

        let result = reconcile_verification_policy(
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

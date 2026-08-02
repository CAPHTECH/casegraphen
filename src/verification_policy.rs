//! Experimental verification-policy reconciliation.
//!
//! This module distinguishes ledger-observable identity/capability facts,
//! runtime attestations, and properties CaseGraphen cannot observe. A policy
//! result is not an evidence acceptance or a proof of independent minds.

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

/// A deterministic world-anchor observation. It is still only an input to the
/// policy reconciler; normal CaseGraphen evidence review remains authoritative.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnchorObservation {
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

impl AnchorObservation {
    fn id(&self) -> &str {
        match self {
            Self::SourceArtifactHash { anchor_id, .. }
            | Self::ToolObservedTest { anchor_id, .. } => anchor_id,
        }
    }

    fn deterministically_satisfied(&self) -> bool {
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
    anchors: &[AnchorObservation],
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
            .is_some_and(|anchor| anchor.deterministically_satisfied())
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

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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

    fn anchor() -> AnchorObservation {
        AnchorObservation::SourceArtifactHash {
            anchor_id: "anchor:source".into(),
            expected_sha256: HASH.into(),
            observed_sha256: HASH.into(),
        }
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
        let bad_anchor = AnchorObservation::ToolObservedTest {
            anchor_id: "anchor:source".into(),
            command_hash: HASH.into(),
            exit_code: 1,
        };
        let result = reconcile_verification_policy(
            &policy(),
            &producer(),
            &[verifier(
                "review:1",
                "actor:v1",
                VerifierDisposition::Accept,
            )],
            &[bad_anchor],
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
        let anchors = [
            anchor(),
            AnchorObservation::SourceArtifactHash {
                anchor_id: "anchor:source".into(),
                expected_sha256: HASH.into(),
                observed_sha256: "f".repeat(64),
            },
        ];
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
}

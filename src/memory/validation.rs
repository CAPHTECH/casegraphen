use super::{
    authority::{origin_ceiling, provenance_role_ceiling},
    temporal::{validate_timestamp, validate_valid_time},
    MemoryClaim, MemoryClaimProposal, MemoryKind, MemoryPolicy, MemoryProjection, MemoryQuery,
    MemorySourceKind, MemoryUseReport, MemoryValidationFinding, ProvenanceRole, SourceRecord,
    MEMORY_CLAIM_PROPOSAL_SCHEMA, MEMORY_CLAIM_SCHEMA, MEMORY_POLICY_SCHEMA, MEMORY_QUERY_SCHEMA,
    MEMORY_SOURCE_RECORD_SCHEMA, MEMORY_USE_REPORT_SCHEMA,
};
use crate::{
    evidence_trust::EvidenceTrustBoundary,
    native_model::{CaseCell, CaseCellLifecycle, CaseCellType},
};
use higher_graphen_core::{Confidence, Id, Provenance, ReviewStatus, SourceKind, SourceRef};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub fn parse_memory_source_record(input: &str) -> Result<SourceRecord, serde_json::Error> {
    parse(input)
}

pub fn parse_memory_claim(input: &str) -> Result<MemoryClaim, serde_json::Error> {
    parse(input)
}

pub fn parse_memory_query(input: &str) -> Result<MemoryQuery, serde_json::Error> {
    parse(input)
}

pub fn parse_memory_policy(input: &str) -> Result<MemoryPolicy, serde_json::Error> {
    parse(input)
}

pub fn parse_memory_use_report(input: &str) -> Result<MemoryUseReport, serde_json::Error> {
    parse(input)
}

fn parse<T: DeserializeOwned>(input: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(input)
}

pub fn validate_memory_source_record(
    source: &SourceRecord,
    artifact_bytes: &[u8],
) -> Vec<MemoryValidationFinding> {
    let mut findings = validate_memory_source_record_contract(source);
    let actual = sha256(artifact_bytes);
    if let Some(expected) = source.content_hash.strip_prefix("sha256:") {
        if is_sha256(expected) && expected != actual {
            findings.push(finding(
                "source_content_hash_mismatch",
                "$.content_hash",
                "source record hash does not match the exact artifact bytes",
            ));
        }
    }
    findings
}

pub(crate) fn validate_memory_source_record_contract(
    source: &SourceRecord,
) -> Vec<MemoryValidationFinding> {
    let mut findings = Vec::new();
    require_schema(
        &source.schema,
        MEMORY_SOURCE_RECORD_SCHEMA,
        "$.schema",
        &mut findings,
    );
    for (location, value) in [
        ("$.source_record_id", source.source_record_id.as_str()),
        ("$.origin_actor_id", source.origin_actor_id.as_str()),
        ("$.source_boundary_id", source.source_boundary_id.as_str()),
        ("$.artifact_ref", source.artifact_ref.as_str()),
    ] {
        require_non_empty(value, location, &mut findings);
    }
    validate_timestamp(&source.captured_at, "$.captured_at", &mut findings);
    match source.content_hash.strip_prefix("sha256:") {
        Some(expected) if is_sha256(expected) => {}
        _ => findings.push(finding(
            "invalid_source_content_hash",
            "$.content_hash",
            "content_hash must be sha256:<64 lowercase hex characters>",
        )),
    }
    findings
}

pub fn validate_memory_claim(
    claim: &MemoryClaim,
    policy: Option<&MemoryPolicy>,
) -> Vec<MemoryValidationFinding> {
    let mut findings = Vec::new();
    require_schema(
        &claim.schema,
        MEMORY_CLAIM_SCHEMA,
        "$.schema",
        &mut findings,
    );
    for (location, value) in [
        ("$.claim_id", claim.claim_id.as_str()),
        ("$.statement.predicate", claim.statement.predicate.as_str()),
        ("$.derivation_actor_id", claim.derivation_actor_id.as_str()),
        ("$.derivation_method", claim.derivation_method.as_str()),
    ] {
        require_non_empty(value, location, &mut findings);
    }
    if claim.subject_refs.is_empty() {
        findings.push(finding(
            "missing_subject",
            "$.subject_refs",
            "a reusable memory claim must name at least one subject",
        ));
    }
    if claim.source_refs.is_empty() {
        findings.push(finding(
            "missing_source",
            "$.source_refs",
            "a memory claim must cite immutable source evidence",
        ));
    }
    if !claim.model_assertions_are_untrusted {
        findings.push(finding(
            "model_trust_assertion_refused",
            "$.model_assertions_are_untrusted",
            "model assertions must remain untrusted until the existing review path accepts them",
        ));
    }
    validate_valid_time(&claim.valid_time, "$.valid_time", &mut findings);
    let required_kinds = policy.map_or_else(
        || {
            vec![
                MemoryKind::Preference,
                MemoryKind::Goal,
                MemoryKind::Commitment,
            ]
        },
        |policy| policy.valid_time_required_kinds.clone(),
    );
    if required_kinds.contains(&claim.memory_kind) && claim.valid_time.valid_from.is_none() {
        findings.push(finding(
            "valid_time_required",
            "$.valid_time.valid_from",
            "this memory kind changes over time and requires valid_from",
        ));
    }
    if claim.authority_ceiling > provenance_role_ceiling(claim.provenance_role) {
        findings.push(finding(
            "authority_amplification",
            "$.authority_ceiling",
            "claim authority exceeds the provenance role ceiling without an authorized elevation binding",
        ));
    }
    if policy
        .is_some_and(|policy| claim.scope.project_id.as_deref() != Some(policy.project_id.as_str()))
    {
        findings.push(finding(
            "claim_project_outside_policy",
            "$.scope.project_id",
            "claim project must equal the governing memory policy project",
        ));
    }
    findings
}

pub fn validate_memory_proposal(
    source: &SourceRecord,
    claim: &MemoryClaim,
    artifact_bytes: &[u8],
) -> Vec<MemoryValidationFinding> {
    let mut findings = validate_memory_source_record(source, artifact_bytes);
    findings.extend(validate_memory_claim(claim, None));
    let artifact_id = format!("artifact:sha256-{}", sha256(artifact_bytes));
    if !claim.source_refs.contains(&artifact_id) {
        findings.push(finding(
            "claim_source_artifact_mismatch",
            "$.source_refs",
            "claim must cite the content-addressed artifact produced from the supplied bytes",
        ));
    }
    if claim.authority_ceiling > origin_ceiling(source.authority_origin) {
        findings.push(finding(
            "authority_amplification",
            "$.authority_ceiling",
            "claim authority exceeds the source-record authority origin ceiling",
        ));
    }
    if claim.sensitivity < source.sensitivity {
        findings.push(finding(
            "sensitivity_downgrade",
            "$.sensitivity",
            "claim sensitivity cannot be lower than its source record sensitivity",
        ));
    }
    if !source_role_is_preserved(source.authority_origin, claim.provenance_role) {
        findings.push(finding(
            "provenance_role_mismatch",
            "$.provenance_role",
            "claim provenance role must preserve the source record authority origin",
        ));
    }
    findings.sort_by(|left, right| {
        (&left.code, &left.location, &left.detail).cmp(&(
            &right.code,
            &right.location,
            &right.detail,
        ))
    });
    findings.dedup();
    findings
}

pub fn build_claim_proposal(
    source: &SourceRecord,
    claim: &MemoryClaim,
    artifact_bytes: &[u8],
    space_id: &Id,
) -> Result<MemoryClaimProposal, Vec<MemoryValidationFinding>> {
    let findings = validate_memory_proposal(source, claim, artifact_bytes);
    let claim_id = Id::new(claim.claim_id.clone()).map_err(|error| {
        vec![finding(
            "invalid_claim_id",
            "$.claim_id",
            &error.to_string(),
        )]
    })?;
    let source_id = Id::new(source.source_record_id.clone()).map_err(|error| {
        vec![finding(
            "invalid_source_record_id",
            "$.source_record_id",
            &error.to_string(),
        )]
    })?;
    let digest = sha256(artifact_bytes);
    let artifact_id = Id::new(format!("artifact:sha256-{digest}")).expect("sha256 id is valid");
    if !findings.is_empty() {
        return Err(findings);
    }
    let mut source_ref = SourceRef::new(source_kind(claim.provenance_role, source.source_kind));
    source_ref.uri = Some(source.artifact_ref.clone());
    source_ref.captured_at = Some(source.captured_at.clone());
    source_ref.source_local_id = Some(source.source_record_id.clone());
    let provenance = Provenance::new(source_ref, Confidence::new(0.0).expect("zero confidence"))
        .with_review_status(ReviewStatus::Unreviewed);
    let metadata = Map::from_iter([
        (
            "memory_claim".to_owned(),
            serde_json::to_value(claim).expect("typed memory claim serializes"),
        ),
        (
            "memory_source_record_id".to_owned(),
            Value::String(source.source_record_id.clone()),
        ),
        (
            "memory_source_records".to_owned(),
            serde_json::to_value([source]).expect("typed memory source record serializes"),
        ),
        (
            "proposed_evidence_boundary".to_owned(),
            Value::String(EvidenceTrustBoundary::Inferred.metadata_value().to_owned()),
        ),
    ]);
    Ok(MemoryClaimProposal {
        schema: MEMORY_CLAIM_PROPOSAL_SCHEMA.to_owned(),
        claim_cell: CaseCell {
            id: claim_id,
            cell_type: CaseCellType::Evidence,
            space_id: space_id.clone(),
            title: format!("Memory claim: {}", claim.statement.predicate),
            summary: Some(claim.statement.object.to_string()),
            lifecycle: CaseCellLifecycle::Proposed,
            source_ids: vec![source_id],
            structure_ids: Vec::new(),
            provenance,
            metadata,
        },
        source_artifact_id: artifact_id,
        findings: Vec::new(),
        accepted: false,
        mutation_performed: false,
    })
}

pub fn validate_memory_query(
    query: &MemoryQuery,
    policy: &MemoryPolicy,
) -> Vec<MemoryValidationFinding> {
    let mut findings = Vec::new();
    require_schema(
        &query.schema,
        MEMORY_QUERY_SCHEMA,
        "$.schema",
        &mut findings,
    );
    for (location, value) in [
        ("$.query_id", query.query_id.as_str()),
        ("$.base_revision_id", query.base_revision_id.as_str()),
        ("$.requesting_actor_id", query.requesting_actor_id.as_str()),
        ("$.purpose", query.purpose.as_str()),
        ("$.risk_class", query.risk_class.as_str()),
    ] {
        require_non_empty(value, location, &mut findings);
    }
    validate_timestamp(&query.as_of, "$.as_of", &mut findings);
    if query.budget.max_items == 0 {
        findings.push(finding(
            "invalid_item_budget",
            "$.budget.max_items",
            "max_items must be at least one",
        ));
    }
    if query.budget.max_tokens == 0 {
        findings.push(finding(
            "invalid_token_budget",
            "$.budget.max_tokens",
            "max_tokens must be at least one",
        ));
    }
    if query.scope.project_id.as_deref() != Some(policy.project_id.as_str()) {
        findings.push(finding(
            "query_project_outside_policy",
            "$.scope.project_id",
            "query project must equal the governing memory policy project",
        ));
    }
    findings
}

pub fn validate_memory_policy(policy: &MemoryPolicy) -> Vec<MemoryValidationFinding> {
    let mut findings = Vec::new();
    require_schema(
        &policy.schema,
        MEMORY_POLICY_SCHEMA,
        "$.schema",
        &mut findings,
    );
    require_non_empty(&policy.policy_id, "$.policy_id", &mut findings);
    require_non_empty(&policy.project_id, "$.project_id", &mut findings);
    let mut actor_ids = BTreeSet::new();
    for (index, grant) in policy.actor_grants.iter().enumerate() {
        if !actor_ids.insert(grant.actor_id.as_str()) {
            findings.push(finding(
                "duplicate_actor_grant",
                &format!("$.actor_grants[{index}].actor_id"),
                "one policy may declare an actor only once",
            ));
        }
        if grant.allowed_audiences.is_empty()
            || grant.allowed_purposes.is_empty()
            || grant.project_ids.is_empty()
        {
            findings.push(finding(
                "empty_actor_grant",
                &format!("$.actor_grants[{index}]"),
                "actor grants require audience, purpose, and project restrictions",
            ));
        }
    }
    for (index, relation_type) in policy.hard_conflict_relation_types.iter().enumerate() {
        if relation_type != "contradicts" {
            findings.push(finding(
                "unsupported_hard_conflict_relation_type",
                &format!("$.hard_conflict_relation_types[{index}]"),
                "v0 classifies only accepted contradicts relations as conflicts",
            ));
        }
    }
    findings
}

pub fn validate_memory_use_report(
    report: &MemoryUseReport,
    projection: &MemoryProjection,
) -> Vec<MemoryValidationFinding> {
    let mut findings = Vec::new();
    require_schema(
        &report.schema,
        MEMORY_USE_REPORT_SCHEMA,
        "$.schema",
        &mut findings,
    );
    require_non_empty(&report.action_id, "$.action_id", &mut findings);
    for (field, ids) in [
        ("cited_claim_ids", &report.cited_claim_ids),
        ("ignored_constraint_ids", &report.ignored_constraint_ids),
    ] {
        let mut seen = BTreeSet::new();
        for (index, id) in ids.iter().enumerate() {
            require_non_empty(id, &format!("$.{field}[{index}]"), &mut findings);
            if !seen.insert(id.as_str()) {
                findings.push(finding(
                    "duplicate_use_report_claim_id",
                    &format!("$.{field}[{index}]"),
                    "claim IDs in a memory use report array must be unique",
                ));
            }
        }
    }
    if !report.self_reported {
        findings.push(finding(
            "use_report_must_be_self_reported",
            "$.self_reported",
            "runtime use reports are self-reported observations by definition",
        ));
    }
    if report.accepted {
        findings.push(finding(
            "use_report_claims_acceptance",
            "$.accepted",
            "a memory use report cannot declare itself accepted",
        ));
    }
    if report.projection_content_hash != projection.projection_content_hash {
        findings.push(finding(
            "use_report_projection_hash_mismatch",
            "$.projection_content_hash",
            "use report does not bind the exact projected context",
        ));
    }
    if projection.projection_content_hash != projection_content_hash(projection) {
        findings.push(finding(
            "projection_content_hash_mismatch",
            "$.projection_content_hash",
            "the supplied projection content differs from its declared content hash",
        ));
    }
    let selected = projection
        .selected_claim_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for (field, ids) in [
        ("cited_claim_ids", &report.cited_claim_ids),
        ("ignored_constraint_ids", &report.ignored_constraint_ids),
    ] {
        for id in ids {
            if !selected.contains(id.as_str()) {
                findings.push(finding(
                    "use_report_unknown_claim",
                    &format!("$.{field}"),
                    "use report may cite only claims present in the bound projection",
                ));
            }
        }
    }
    findings
}

pub(crate) fn source_role_is_preserved(
    origin: super::AuthorityOrigin,
    role: ProvenanceRole,
) -> bool {
    match origin {
        super::AuthorityOrigin::External => matches!(
            role,
            ProvenanceRole::ExternalMaterial | ProvenanceRole::UnverifiedThirdPartyStatement
        ),
        super::AuthorityOrigin::Tool => role == ProvenanceRole::ToolObservation,
        super::AuthorityOrigin::Inferred => role == ProvenanceRole::AgentInference,
        super::AuthorityOrigin::User => matches!(
            role,
            ProvenanceRole::UserRequirement | ProvenanceRole::CanonicalHumanStatement
        ),
        super::AuthorityOrigin::Operator => matches!(
            role,
            ProvenanceRole::OperatorInstruction | ProvenanceRole::CanonicalHumanStatement
        ),
        super::AuthorityOrigin::Reviewer => matches!(
            role,
            ProvenanceRole::ReviewedArchitectureDecision | ProvenanceRole::CanonicalHumanStatement
        ),
    }
}

pub(crate) fn projection_content_hash(projection: &MemoryProjection) -> String {
    let mut content = projection.clone();
    content.projection_content_hash.clear();
    let bytes = serde_json::to_vec(&content).expect("typed memory projection serializes");
    format!("sha256:{}", sha256(&bytes))
}

fn source_kind(role: ProvenanceRole, kind: MemorySourceKind) -> SourceKind {
    match role {
        ProvenanceRole::AgentInference => SourceKind::Ai,
        ProvenanceRole::CanonicalHumanStatement
        | ProvenanceRole::OperatorInstruction
        | ProvenanceRole::UserRequirement => SourceKind::Human,
        ProvenanceRole::ToolObservation => SourceKind::Api,
        ProvenanceRole::ExternalMaterial | ProvenanceRole::UnverifiedThirdPartyStatement => {
            SourceKind::External
        }
        ProvenanceRole::ReviewedArchitectureDecision => match kind {
            MemorySourceKind::Document => SourceKind::Document,
            MemorySourceKind::RuntimeTrace | MemorySourceKind::ToolOutput => SourceKind::Log,
            MemorySourceKind::Conversation => SourceKind::Human,
            MemorySourceKind::Artifact => SourceKind::Code,
        },
    }
}

fn require_schema(
    actual: &str,
    expected: &str,
    location: &str,
    findings: &mut Vec<MemoryValidationFinding>,
) {
    if actual != expected {
        findings.push(finding(
            "unsupported_schema",
            location,
            &format!("expected {expected}"),
        ));
    }
}

fn require_non_empty(value: &str, location: &str, findings: &mut Vec<MemoryValidationFinding>) {
    if value.trim().is_empty() {
        findings.push(finding(
            "empty_required_field",
            location,
            "field must not be empty",
        ));
    }
}

pub(crate) fn finding(code: &str, location: &str, detail: &str) -> MemoryValidationFinding {
    MemoryValidationFinding {
        code: code.to_owned(),
        location: location.to_owned(),
        detail: detail.to_owned(),
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

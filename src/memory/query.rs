use super::{
    authority::{claim_in_scope, claim_within_grant, origin_ceiling},
    conflicts::{
        contradictions, has_authority_binding, has_source_relation, retracting_claims,
        superseding_claims,
    },
    temporal::{disposition, TemporalDisposition},
    validation::{
        is_sha256, source_role_is_preserved, validate_memory_claim,
        validate_memory_source_record_contract,
    },
    ActorMemoryGrant, AuthorityLevel, MemoryClaim, MemoryPolicy, MemoryQuery, MemoryStatus,
    ProjectedMemory, Sensitivity, SourceRecord, MEMORY_SOURCE_RECORD_SCHEMA,
};
use crate::{
    evidence_trust::evidence_is_acceptable,
    native_eval::{effective_evidence_review_status, latest_evidence_review_status},
    native_model::{
        native_evidence_trust_input, CaseCell, CaseCellLifecycle, CaseCellType, CaseSpace,
    },
};
use higher_graphen_core::ReviewStatus;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(crate) struct DerivedMemory {
    pub item: ProjectedMemory,
    pub exclusion_reason: Option<String>,
}

pub(crate) fn derive_memory(
    case_space: &CaseSpace,
    query: &MemoryQuery,
    policy: &MemoryPolicy,
    grant: &ActorMemoryGrant,
) -> Vec<DerivedMemory> {
    let parsed = case_space
        .case_cells
        .iter()
        .filter_map(|cell| {
            cell.metadata
                .get("memory_claim")
                .cloned()
                .map(|value| (cell, serde_json::from_value::<MemoryClaim>(value)))
        })
        .collect::<Vec<_>>();
    let mut base = parsed
        .into_iter()
        .map(|(cell, parsed_claim)| match parsed_claim {
            Ok(claim) => derive_base(case_space, cell, claim, query, policy, grant),
            Err(_) => DerivedMemory {
                item: malformed_item(cell),
                exclusion_reason: Some("invalid_claim_contract".to_owned()),
            },
        })
        .collect::<Vec<_>>();

    let accepted_current = base
        .iter()
        .filter(|derived| {
            derived.item.status == MemoryStatus::Accepted
                && derived.exclusion_reason.is_none()
                && disposition(&derived.item.valid_time, &query.as_of)
                    == TemporalDisposition::Current
        })
        .map(|derived| derived.item.claim_id.clone())
        .collect::<BTreeSet<_>>();
    let contested_current = resolve_current_conflicts(
        case_space,
        &accepted_current,
        &policy.hard_conflict_relation_types,
    );
    let contested_current_ids = contested_current.keys().cloned().collect::<BTreeSet<_>>();
    let active_current = accepted_current
        .difference(&contested_current_ids)
        .cloned()
        .collect::<BTreeSet<_>>();

    for derived in &mut base {
        if is_pre_rank_exclusion(derived.exclusion_reason.as_deref()) {
            continue;
        }
        if !matches!(
            derived.item.status,
            MemoryStatus::Accepted | MemoryStatus::Expired
        ) {
            continue;
        }
        let claim_id = derived.item.claim_id.as_str();
        if retracting_claims(case_space, claim_id)
            .any(|source_id| active_current.contains(source_id))
        {
            derived.item.status = MemoryStatus::Retracted;
            derived.exclusion_reason = Some("retracted".to_owned());
            continue;
        }
        if superseding_claims(case_space, claim_id)
            .any(|source_id| active_current.contains(source_id))
        {
            derived.item.status = MemoryStatus::Superseded;
            derived.exclusion_reason = Some("superseded".to_owned());
            continue;
        }
        if let Some(hard) = contested_current.get(claim_id) {
            derived.item.status = MemoryStatus::Contested;
            derived.item.hard_conflict = *hard;
            derived.exclusion_reason = Some(if *hard {
                "hard_conflict".to_owned()
            } else {
                "contested".to_owned()
            });
        }
    }

    base.sort_by(|left, right| left.item.claim_id.cmp(&right.item.claim_id));
    base
}

/// Resolves conflicts to a fixed point because a contested claim cannot hide
/// another claim through supersession or retraction. Removing that suppressor
/// can expose another current conflict, but the contested set only grows over
/// the finite accepted-current set.
fn resolve_current_conflicts(
    case_space: &CaseSpace,
    accepted_current: &BTreeSet<String>,
    hard_relation_names: &[String],
) -> BTreeMap<String, bool> {
    let mut contested = BTreeMap::new();
    loop {
        let contested_ids = contested.keys().cloned().collect::<BTreeSet<_>>();
        let activators = accepted_current
            .difference(&contested_ids)
            .cloned()
            .collect::<BTreeSet<_>>();
        let suppressed = accepted_current
            .iter()
            .filter(|target_id| {
                retracting_claims(case_space, target_id)
                    .chain(superseding_claims(case_space, target_id))
                    .any(|source_id| activators.contains(source_id))
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        let visible = accepted_current
            .difference(&suppressed)
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut next = BTreeMap::<String, bool>::new();
        for claim_id in &visible {
            for (other_id, hard) in contradictions(case_space, claim_id, hard_relation_names) {
                if !visible.contains(other_id) {
                    continue;
                }
                next.entry(claim_id.clone())
                    .and_modify(|current| *current |= hard)
                    .or_insert(hard);
                next.entry(other_id.to_owned())
                    .and_modify(|current| *current |= hard)
                    .or_insert(hard);
            }
        }
        if next == contested {
            return next;
        }
        contested = next;
    }
}

pub(crate) fn is_pre_rank_exclusion(reason: Option<&str>) -> bool {
    matches!(
        reason,
        Some(
            "outside_scope"
                | "outside_authority_grant"
                | "memory_kind_filtered"
                | "unsupported_source"
                | "invalid_claim_contract"
                | "claim_identity_mismatch"
                | "authority_amplification"
                | "provenance_role_mismatch"
                | "sensitivity_downgrade"
        )
    )
}

fn derive_base(
    case_space: &CaseSpace,
    cell: &CaseCell,
    claim: MemoryClaim,
    query: &MemoryQuery,
    policy: &MemoryPolicy,
    grant: &ActorMemoryGrant,
) -> DerivedMemory {
    let mut exclusion_reason = None;
    let latest_review = latest_evidence_review_status(case_space, cell.id.as_str());
    let effective_review = effective_evidence_review_status(case_space, cell);
    let trusted = cell.cell_type == CaseCellType::Evidence
        && evidence_is_acceptable(native_evidence_trust_input(cell, latest_review));
    let mut status = if cell.lifecycle == CaseCellLifecycle::Rejected
        || effective_review == Some(ReviewStatus::Rejected)
    {
        MemoryStatus::Rejected
    } else if trusted && effective_review == Some(ReviewStatus::Accepted) {
        MemoryStatus::Accepted
    } else {
        MemoryStatus::Candidate
    };

    if claim.claim_id != cell.id.as_str() {
        status = MemoryStatus::Candidate;
        exclusion_reason = Some("claim_identity_mismatch".to_owned());
    }
    let mut claim_findings = validate_memory_claim(&claim, Some(policy));
    let authority_binding =
        has_authority_binding(case_space, &claim.claim_id, claim.authority_ceiling);
    if authority_binding {
        claim_findings.retain(|finding| finding.code != "authority_amplification");
    }
    if !claim_findings.is_empty() {
        status = MemoryStatus::Candidate;
        exclusion_reason = Some("invalid_claim_contract".to_owned());
    }
    let source_records = source_records(cell);
    let immutable_sources = source_records
        .as_ref()
        .is_some_and(|records| sources_are_immutable(case_space, &claim, records));
    if !immutable_sources {
        status = MemoryStatus::Candidate;
        exclusion_reason = Some("unsupported_source".to_owned());
    } else if source_records
        .as_ref()
        .and_then(|records| source_authority_ceiling(records))
        .is_some_and(|ceiling| claim.authority_ceiling > ceiling)
        && !authority_binding
    {
        status = MemoryStatus::Candidate;
        exclusion_reason = Some("authority_amplification".to_owned());
    } else if source_records
        .as_ref()
        .and_then(|records| source_sensitivity(records))
        .is_some_and(|sensitivity| claim.sensitivity < sensitivity)
    {
        status = MemoryStatus::Candidate;
        exclusion_reason = Some("sensitivity_downgrade".to_owned());
    } else if source_records.as_ref().is_some_and(|records| {
        records
            .iter()
            .any(|record| !source_role_is_preserved(record.authority_origin, claim.provenance_role))
    }) {
        status = MemoryStatus::Candidate;
        exclusion_reason = Some("provenance_role_mismatch".to_owned());
    }

    match disposition(&claim.valid_time, &query.as_of) {
        TemporalDisposition::Expired if status == MemoryStatus::Accepted => {
            status = MemoryStatus::Expired;
            exclusion_reason = Some("expired".to_owned());
        }
        TemporalDisposition::NotYetValid if status == MemoryStatus::Accepted => {
            exclusion_reason = Some("not_yet_valid".to_owned());
        }
        TemporalDisposition::Current
        | TemporalDisposition::Expired
        | TemporalDisposition::NotYetValid => {}
    }
    if !claim_in_scope(&claim, query) {
        exclusion_reason = Some("outside_scope".to_owned());
    } else if !claim_within_grant(&claim, grant) {
        exclusion_reason = Some("outside_authority_grant".to_owned());
    } else if !query.memory_kinds.is_empty() && !query.memory_kinds.contains(&claim.memory_kind) {
        exclusion_reason = Some("memory_kind_filtered".to_owned());
    } else if status == MemoryStatus::Candidate && exclusion_reason.is_none() {
        exclusion_reason = Some("candidate".to_owned());
    } else if status == MemoryStatus::Rejected {
        exclusion_reason = Some("rejected".to_owned());
    }

    let evidence_strength = if trusted && immutable_sources {
        "source_backed"
    } else {
        "unsupported"
    };
    let relevance_score = relevance_score(&claim, &query.query_text);
    DerivedMemory {
        item: ProjectedMemory {
            claim_id: claim.claim_id,
            memory_kind: claim.memory_kind,
            statement: claim.statement,
            subject_refs: claim.subject_refs,
            source_refs: claim.source_refs,
            status,
            authority: claim.authority_ceiling,
            sensitivity: claim.sensitivity,
            valid_time: claim.valid_time,
            relevance_score,
            evidence_strength: evidence_strength.to_owned(),
            hard_conflict: false,
        },
        exclusion_reason,
    }
}

fn source_records(cell: &CaseCell) -> Option<Vec<SourceRecord>> {
    cell.metadata
        .get("memory_source_records")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

pub fn source_records_for_claim(case_space: &CaseSpace, claim_id: &str) -> Vec<SourceRecord> {
    case_space
        .case_cells
        .iter()
        .find(|cell| cell.id.as_str() == claim_id)
        .and_then(source_records)
        .unwrap_or_default()
}

fn sources_are_immutable(
    case_space: &CaseSpace,
    claim: &MemoryClaim,
    records: &[SourceRecord],
) -> bool {
    !claim.source_refs.is_empty()
        && claim.source_refs.iter().all(|source_id| {
            let source_hash = source_id.strip_prefix("artifact:sha256-");
            let record_matches = source_hash.is_some_and(|hash| {
                records.iter().any(|record| {
                    record.schema == MEMORY_SOURCE_RECORD_SCHEMA
                        && record.content_hash == format!("sha256:{hash}")
                        && validate_memory_source_record_contract(record).is_empty()
                })
            });
            record_matches
                && has_source_relation(case_space, &claim.claim_id, source_id)
                && case_space.case_cells.iter().any(|source| {
                    source.id.as_str() == source_id
                        && source.cell_type == CaseCellType::Custom("artifact".to_owned())
                        && source
                            .metadata
                            .get("content_hash")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(is_sha256)
                        && source_id
                            .strip_prefix("artifact:sha256-")
                            .is_some_and(|id_hash| {
                                source
                                    .metadata
                                    .get("content_hash")
                                    .and_then(serde_json::Value::as_str)
                                    == Some(id_hash)
                            })
                })
        })
}

fn source_sensitivity(records: &[SourceRecord]) -> Option<Sensitivity> {
    records.iter().map(|record| record.sensitivity).max()
}

fn source_authority_ceiling(records: &[SourceRecord]) -> Option<AuthorityLevel> {
    records
        .iter()
        .map(|record| origin_ceiling(record.authority_origin))
        .min()
}

fn relevance_score(claim: &MemoryClaim, query_text: &str) -> u64 {
    let query_terms = terms(query_text);
    if query_terms.is_empty() {
        return 0;
    }
    let material = format!(
        "{} {} {}",
        claim.statement.predicate,
        claim.statement.object,
        claim.subject_refs.join(" ")
    );
    let claim_terms = terms(&material).into_iter().collect::<BTreeSet<_>>();
    query_terms
        .into_iter()
        .map(|term| u64::from(claim_terms.contains(&term)))
        .sum()
}

pub(crate) fn lexical_terms(item: &ProjectedMemory) -> Vec<String> {
    let material = format!(
        "{} {} {}",
        item.statement.predicate,
        item.statement.object,
        item.subject_refs.join(" ")
    );
    terms(&material)
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn terms(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn malformed_item(cell: &CaseCell) -> ProjectedMemory {
    ProjectedMemory {
        claim_id: cell.id.to_string(),
        memory_kind: super::MemoryKind::Observation,
        statement: super::MemoryStatement {
            predicate: "invalid_memory_claim".to_owned(),
            object: serde_json::Value::Null,
        },
        subject_refs: Vec::new(),
        source_refs: Vec::new(),
        status: MemoryStatus::Candidate,
        authority: super::AuthorityLevel::Untrusted,
        sensitivity: super::Sensitivity::Restricted,
        valid_time: super::ValidTime::default(),
        relevance_score: 0,
        evidence_strength: "invalid".to_owned(),
        hard_conflict: false,
    }
}

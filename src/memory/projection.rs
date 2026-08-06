use super::{
    authority::actor_grant,
    query::{derive_memory, is_pre_rank_exclusion, DerivedMemory},
    validation::{finding, projection_content_hash, validate_memory_policy, validate_memory_query},
    MemoryOmission, MemoryPolicy, MemoryProjection, MemoryProjectionLoss, MemoryQuery,
    MemoryStatus, MemoryValidationFinding, MEMORY_PROJECTION_SCHEMA,
};
use crate::{native_hash::sha256_hex, native_model::CaseSpace};
use std::collections::{BTreeMap, BTreeSet};

pub fn query_memory(
    case_space: &CaseSpace,
    query: &MemoryQuery,
    policy: &MemoryPolicy,
) -> Result<MemoryProjection, Vec<MemoryValidationFinding>> {
    let mut findings = validate_memory_policy(policy);
    findings.extend(validate_memory_query(query, policy));
    if case_space.revision.revision_id.as_str() != query.base_revision_id {
        findings.push(finding(
            "stale_base_revision",
            "$.base_revision_id",
            "memory query must bind the exact replayed CaseSpace revision",
        ));
    }
    let Some(grant) = actor_grant(policy, query) else {
        findings.push(finding(
            "memory_query_not_authorized",
            "$.requesting_actor_id",
            "no actor grant permits this audience, purpose, and project",
        ));
        return Err(sorted_findings(findings));
    };
    if !findings.is_empty() {
        return Err(sorted_findings(findings));
    }

    let derived = derive_memory(case_space, query, policy, grant);
    let contested_claim_ids = derived
        .iter()
        .filter(|item| item.item.status == MemoryStatus::Contested)
        .map(|item| item.item.claim_id.clone())
        .collect::<Vec<_>>();
    let mut eligible = Vec::new();
    let mut omissions = Vec::new();
    for derived in derived {
        if included_by_status(&derived, query) {
            eligible.push(derived.item);
        } else {
            omissions.push(MemoryOmission {
                claim_id: derived.item.claim_id,
                reason: derived
                    .exclusion_reason
                    .unwrap_or_else(|| status_name(derived.item.status).to_owned()),
            });
        }
    }
    eligible.sort_by(|left, right| {
        right
            .relevance_score
            .cmp(&left.relevance_score)
            .then_with(|| left.claim_id.cmp(&right.claim_id))
    });

    let mut selected = Vec::new();
    let mut consumed_tokens = 0_usize;
    for item in eligible {
        if selected.len() >= query.budget.max_items {
            omissions.push(MemoryOmission {
                claim_id: item.claim_id,
                reason: "item_budget".to_owned(),
            });
            continue;
        }
        let item_tokens = estimated_tokens(&item);
        if consumed_tokens.saturating_add(item_tokens) > query.budget.max_tokens {
            omissions.push(MemoryOmission {
                claim_id: item.claim_id,
                reason: "token_budget".to_owned(),
            });
            continue;
        }
        consumed_tokens += item_tokens;
        selected.push(item);
    }
    omissions.sort_by(|left, right| {
        (&left.claim_id, &left.reason).cmp(&(&right.claim_id, &right.reason))
    });
    let losses = projection_losses(&omissions);
    let selected_claim_ids = selected
        .iter()
        .map(|item| item.claim_id.clone())
        .collect::<Vec<_>>();
    let source_refs = selected
        .iter()
        .flat_map(|item| item.source_refs.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let authority_summary = selected.iter().fold(BTreeMap::new(), |mut counts, item| {
        *counts.entry(item.authority).or_insert(0) += 1;
        counts
    });
    let query_hash = hash(query);
    let mut projection = MemoryProjection {
        schema: MEMORY_PROJECTION_SCHEMA.to_owned(),
        projection_id: format!(
            "memory-projection:{}",
            query_hash.trim_start_matches("sha256:")
        ),
        base_revision_id: query.base_revision_id.clone(),
        query_hash,
        audience: query.audience,
        selected_claim_ids,
        source_refs,
        contested_claim_ids,
        omissions,
        losses,
        authority_summary,
        temporal_cutoff: query.as_of.clone(),
        token_budget: query.budget.max_tokens,
        projection_content_hash: String::new(),
        items: selected,
        read_only: true,
        accepted_state_changed: false,
    };
    projection.projection_content_hash = projection_content_hash(&projection);
    Ok(projection)
}

fn included_by_status(derived: &DerivedMemory, query: &MemoryQuery) -> bool {
    if is_pre_rank_exclusion(derived.exclusion_reason.as_deref()) {
        return false;
    }
    match derived.item.status {
        MemoryStatus::Accepted => {
            derived.exclusion_reason.as_deref() != Some("not_yet_valid") || query.include_historical
        }
        MemoryStatus::Contested => query.include_contested,
        MemoryStatus::Superseded
        | MemoryStatus::Expired
        | MemoryStatus::Retracted
        | MemoryStatus::Rejected
        | MemoryStatus::Candidate => query.include_historical,
    }
}

fn projection_losses(omissions: &[MemoryOmission]) -> Vec<MemoryProjectionLoss> {
    let grouped = omissions.iter().fold(
        BTreeMap::<String, Vec<String>>::new(),
        |mut grouped, omission| {
            grouped
                .entry(omission.reason.clone())
                .or_default()
                .push(omission.claim_id.clone());
            grouped
        },
    );
    grouped
        .into_iter()
        .map(|(loss_kind, mut omitted_claim_ids)| {
            omitted_claim_ids.sort();
            MemoryProjectionLoss {
                detail: format!(
                    "{} claim(s) omitted by {loss_kind}",
                    omitted_claim_ids.len()
                ),
                loss_kind,
                omitted_claim_ids,
            }
        })
        .collect()
}

fn estimated_tokens(item: &super::ProjectedMemory) -> usize {
    serde_json::to_vec(item)
        .map(|bytes| bytes.len().div_ceil(4))
        .unwrap_or(usize::MAX)
}

fn status_name(status: MemoryStatus) -> &'static str {
    match status {
        MemoryStatus::Candidate => "candidate",
        MemoryStatus::Accepted => "accepted",
        MemoryStatus::Contested => "contested",
        MemoryStatus::Superseded => "superseded",
        MemoryStatus::Expired => "expired",
        MemoryStatus::Retracted => "retracted",
        MemoryStatus::Rejected => "rejected",
    }
}

pub(crate) fn hash(value: &impl serde::Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("typed memory contract serializes");
    format!("sha256:{}", sha256_hex(&bytes))
}

fn sorted_findings(mut findings: Vec<MemoryValidationFinding>) -> Vec<MemoryValidationFinding> {
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

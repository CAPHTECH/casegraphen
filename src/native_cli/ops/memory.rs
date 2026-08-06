use super::{report, NativeCliError, NativeCommandResult};
use crate::{
    memory::{
        build_claim_proposal, parse_memory_claim, parse_memory_policy, parse_memory_query,
        parse_memory_source_record, query_memory, rebuild_memory_index, source_records_for_claim,
        validate_memory_claim, validate_memory_index, validate_memory_policy,
        validate_memory_proposal, validate_memory_source_record, MemoryIndex,
        MemoryValidationFinding,
    },
    native_store::NativeCaseStore,
};
use higher_graphen_core::Id;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{fs, path::Path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryReadMode {
    Query,
    Explain,
    History,
    Conflicts,
    Candidates,
    Sources,
}

impl MemoryReadMode {
    fn command(self) -> &'static str {
        match self {
            Self::Query => "casegraphen memory query",
            Self::Explain => "casegraphen memory explain",
            Self::History => "casegraphen memory history",
            Self::Conflicts => "casegraphen memory conflicts",
            Self::Candidates => "casegraphen memory candidates",
            Self::Sources => "casegraphen memory sources",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemorySourceMode {
    Attach,
    Inspect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryIndexMode {
    Rebuild,
    Validate,
}

pub(in crate::native_cli) fn memory_read(
    store: &Path,
    case_space_id: &Id,
    query_path: &Path,
    policy_path: &Path,
    mode: MemoryReadMode,
    target_id: Option<&Id>,
) -> Result<NativeCommandResult<Value>, NativeCliError> {
    let mut query = parse_memory_query(&read_text(query_path)?)?;
    let policy = parse_memory_policy(&read_text(policy_path)?)?;
    if matches!(
        mode,
        MemoryReadMode::Explain | MemoryReadMode::History | MemoryReadMode::Candidates
    ) {
        query.include_historical = true;
    }
    if matches!(
        mode,
        MemoryReadMode::Explain
            | MemoryReadMode::History
            | MemoryReadMode::Conflicts
            | MemoryReadMode::Candidates
    ) {
        query.include_contested = true;
    }
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let projection =
        query_memory(&replay.case_space, &query, &policy).map_err(NativeCliError::Memory)?;
    let result = match mode {
        MemoryReadMode::Query | MemoryReadMode::Candidates => json!({
            "projection": projection,
            "mutation_performed": false
        }),
        MemoryReadMode::Conflicts => json!({
            "contested_claim_ids": projection.contested_claim_ids,
            "items": projection.items.into_iter().filter(|item| item.hard_conflict || item.status == crate::memory::MemoryStatus::Contested).collect::<Vec<_>>(),
            "losses": projection.losses,
            "base_revision_id": projection.base_revision_id,
            "mutation_performed": false
        }),
        MemoryReadMode::Explain | MemoryReadMode::History => {
            let target = target_id.expect("parser requires --target-id").as_str();
            json!({
                "claim_id": target,
                "item": projection.items.iter().find(|item| item.claim_id == target),
                "omissions": projection.omissions.iter().filter(|item| item.claim_id == target).collect::<Vec<_>>(),
                "contested": projection.contested_claim_ids.iter().any(|id| id == target),
                "base_revision_id": projection.base_revision_id,
                "projection_content_hash": projection.projection_content_hash,
                "mutation_performed": false
            })
        }
        MemoryReadMode::Sources => {
            let target = target_id.expect("parser requires --target-id").as_str();
            let item = projection.items.iter().find(|item| item.claim_id == target);
            let source_records = item
                .is_some()
                .then(|| source_records_for_claim(&replay.case_space, target))
                .unwrap_or_default();
            json!({
                "claim_id": target,
                "source_refs": item.map(|item| item.source_refs.clone()).unwrap_or_default(),
                "source_records": source_records,
                "omissions": projection.omissions.iter().filter(|item| item.claim_id == target).collect::<Vec<_>>(),
                "base_revision_id": projection.base_revision_id,
                "projection_content_hash": projection.projection_content_hash,
                "mutation_performed": false
            })
        }
    };
    Ok(NativeCommandResult::success(report(mode.command(), result)))
}

pub(in crate::native_cli) fn memory_source(
    source_path: &Path,
    artifact_path: &Path,
    mode: MemorySourceMode,
) -> Result<NativeCommandResult<Value>, NativeCliError> {
    let source = parse_memory_source_record(&read_text(source_path)?)?;
    let bytes = read_bytes(artifact_path)?;
    let findings = validate_memory_source_record(&source, &bytes);
    let valid = findings.is_empty();
    let command = match mode {
        MemorySourceMode::Attach => "casegraphen memory source attach",
        MemorySourceMode::Inspect => "casegraphen memory source inspect",
    };
    let result = json!({
        "source_record": source,
        "artifact_id": format!("artifact:sha256-{}", crate::memory::content_hash(&bytes)),
        "valid": valid,
        "findings": findings,
        "accepted": false,
        "mutation_performed": false
    });
    Ok(NativeCommandResult::with_domain_finding(
        report(command, result),
        !valid,
    ))
}

pub(in crate::native_cli) fn memory_check(
    claim_path: &Path,
    source_path: &Path,
    artifact_path: &Path,
    policy_path: &Path,
) -> Result<NativeCommandResult<Value>, NativeCliError> {
    let claim = parse_memory_claim(&read_text(claim_path)?)?;
    let source = parse_memory_source_record(&read_text(source_path)?)?;
    let policy = parse_memory_policy(&read_text(policy_path)?)?;
    let bytes = read_bytes(artifact_path)?;
    let mut findings = validate_memory_policy(&policy);
    findings.extend(validate_memory_claim(&claim, Some(&policy)));
    findings.extend(validate_memory_proposal(&source, &claim, &bytes));
    sort_findings(&mut findings);
    let valid = findings.is_empty();
    Ok(NativeCommandResult::with_domain_finding(
        report(
            "casegraphen memory check",
            json!({
                "valid": valid,
                "findings": findings,
                "accepted": false,
                "mutation_performed": false
            }),
        ),
        !valid,
    ))
}

pub(in crate::native_cli) fn memory_propose(
    claim_path: &Path,
    source_path: &Path,
    artifact_path: &Path,
    policy_path: &Path,
    space_id: &Id,
) -> Result<NativeCommandResult<Value>, NativeCliError> {
    let claim = parse_memory_claim(&read_text(claim_path)?)?;
    let source = parse_memory_source_record(&read_text(source_path)?)?;
    let policy = parse_memory_policy(&read_text(policy_path)?)?;
    let bytes = read_bytes(artifact_path)?;
    let mut findings = validate_memory_policy(&policy);
    findings.extend(validate_memory_claim(&claim, Some(&policy)));
    findings.extend(validate_memory_proposal(&source, &claim, &bytes));
    sort_findings(&mut findings);
    if !findings.is_empty() {
        return Err(NativeCliError::Memory(findings));
    }
    let proposal =
        build_claim_proposal(&source, &claim, &bytes, space_id).map_err(NativeCliError::Memory)?;
    Ok(NativeCommandResult::success(report(
        "casegraphen memory propose",
        serde_json::to_value(proposal)?,
    )))
}

pub(in crate::native_cli) fn memory_index(
    store: &Path,
    case_space_id: &Id,
    query_path: &Path,
    policy_path: &Path,
    mode: MemoryIndexMode,
    index_path: Option<&Path>,
) -> Result<NativeCommandResult<Value>, NativeCliError> {
    let query = parse_memory_query(&read_text(query_path)?)?;
    let policy = parse_memory_policy(&read_text(policy_path)?)?;
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let rebuilt = rebuild_memory_index(&replay.case_space, &query, &policy)
        .map_err(NativeCliError::Memory)?;
    match mode {
        MemoryIndexMode::Rebuild => Ok(NativeCommandResult::success(report(
            "casegraphen memory index rebuild",
            json!({"index": rebuilt, "mutation_performed": false}),
        ))),
        MemoryIndexMode::Validate => {
            let path = index_path.expect("parser requires --index");
            let actual: MemoryIndex = read_json(path)?;
            let validation = validate_memory_index(&replay.case_space, &query, &policy, &actual);
            let valid = validation.valid;
            Ok(NativeCommandResult::with_domain_finding(
                report(
                    "casegraphen memory index validate",
                    json!({"validation": validation, "mutation_performed": false}),
                ),
                !valid,
            ))
        }
    }
}

fn read_text(path: &Path) -> Result<String, NativeCliError> {
    fs::read_to_string(path).map_err(|source| NativeCliError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, NativeCliError> {
    fs::read(path).map_err(|source| NativeCliError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, NativeCliError> {
    Ok(serde_json::from_str(&read_text(path)?)?)
}

fn sort_findings(findings: &mut Vec<MemoryValidationFinding>) {
    findings.sort_by(|left, right| {
        (&left.code, &left.location, &left.detail).cmp(&(
            &right.code,
            &right.location,
            &right.detail,
        ))
    });
    findings.dedup();
}

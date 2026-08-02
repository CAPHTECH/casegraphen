//! Experimental, runtime-neutral report and completeness contracts.
//!
//! Runtime declarations are observations at an untrusted boundary.  This
//! module validates their shape and reconciles them with an independently
//! supplied graph expectation; it does not accept evidence, review claims, or
//! case transitions.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Schema identity for the experimental external-runtime node report.
pub const RUNTIME_NODE_REPORT_SCHEMA: &str = "casegraphen.experimental.runtime.node_report.v0";
/// Schema version for the experimental external-runtime node report.
pub const RUNTIME_NODE_REPORT_SCHEMA_VERSION: u32 = 0;
/// Trust marker required on every external-runtime report.
pub const RUNTIME_REPORT_TRUST_BOUNDARY: &str =
    "runtime_reported_untrusted_until_independently_validated_and_reviewed";

/// A runtime's claimed terminal status.  It is not a CaseGraphen acceptance status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeNodeStatus {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

/// Runtime-reported failure classification.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFailureKind {
    ExecutionError,
    Timeout,
    Cancelled,
    ResourceExhausted,
    InvalidOutput,
    Unknown,
}

/// The runtime identity as declared by the runtime itself.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportedRuntimeIdentity {
    pub runtime_name: String,
    pub runtime_version: String,
    pub adapter_name: String,
    pub adapter_version: String,
}

/// Runtime-reported token accounting.  No field is independently anchored here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportedTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

/// Runtime-reported monetary cost.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportedCost {
    pub amount: f64,
    pub currency: String,
}

/// A resource allocation as declared by the runtime.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportedResourceAllocation {
    pub resource_id: String,
    pub mode: String,
    pub allocation_id: String,
}

/// One external runtime attempt.  The whole value remains untrusted input.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeNodeReport {
    pub schema: String,
    pub schema_version: u32,
    pub report_id: String,
    pub runtime_graph_id: String,
    pub runtime_graph_content_hash: String,
    pub node_id: String,
    pub attempt_id: String,
    pub retry_of_attempt_id: Option<String>,
    pub round_id: String,
    pub parent_node_ids: Vec<String>,
    pub input_artifact_ids: Vec<String>,
    pub output_artifact_ids: Vec<String>,
    pub expected_output_schema_id: String,
    pub actual_output_schema_id: Option<String>,
    pub started_at: String,
    pub finished_at: String,
    pub status: RuntimeNodeStatus,
    pub failure_kind: Option<RuntimeFailureKind>,
    pub runtime_identity: ReportedRuntimeIdentity,
    pub reported_model: Option<String>,
    pub reported_context_id: Option<String>,
    pub token_usage: Option<ReportedTokenUsage>,
    pub cost: Option<ReportedCost>,
    pub resource_allocations: Vec<ReportedResourceAllocation>,
    pub worktree_id: Option<String>,
    pub commit_sha: Option<String>,
    pub verifier_report_ids: Vec<String>,
    pub trust_boundary: String,
}

/// One independently supplied node expectation from `execution.topology.v0`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedRuntimeNode {
    pub node_id: String,
    pub expected_output_schema_id: String,
}

/// The content-addressed graph boundary against which reports are reconciled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeGraphExpectation {
    pub runtime_graph_id: String,
    pub runtime_graph_content_hash: String,
    pub nodes: Vec<ExpectedRuntimeNode>,
}

/// Stable diagnostic emitted by validation or reconciliation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeProtocolFinding {
    pub code: String,
    pub node_id: Option<String>,
    pub attempt_id: Option<String>,
    pub detail: String,
}

/// Deterministically derived run-level completeness counters and findings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCompleteness {
    pub expected_node_count: u64,
    pub actual_report_count: u64,
    pub failed_node_count: u64,
    pub missing_report_count: u64,
    pub duplicate_attempt_count: u64,
    pub unaccounted_artifact_count: u64,
    pub complete: bool,
    pub findings: Vec<RuntimeProtocolFinding>,
}

/// Parses and semantically validates a node report.
pub fn parse_runtime_node_report(
    input: &str,
) -> Result<RuntimeNodeReport, Vec<RuntimeProtocolFinding>> {
    let report: RuntimeNodeReport = serde_json::from_str(input)
        .map_err(|error| vec![finding("invalid_json", None, None, error.to_string())])?;
    let findings = validate_runtime_node_report(&report);
    if findings.is_empty() {
        Ok(report)
    } else {
        Err(findings)
    }
}

/// Validates constraints that JSON Schema alone cannot express.
pub fn validate_runtime_node_report(report: &RuntimeNodeReport) -> Vec<RuntimeProtocolFinding> {
    let mut findings = Vec::new();
    if report.schema != RUNTIME_NODE_REPORT_SCHEMA {
        findings.push(report_finding(
            report,
            "unsupported_schema",
            "schema identity does not match runtime.node_report.v0",
        ));
    }
    if report.schema_version != RUNTIME_NODE_REPORT_SCHEMA_VERSION {
        findings.push(report_finding(
            report,
            "unsupported_schema_version",
            "schema_version must be 0",
        ));
    }
    if report.trust_boundary != RUNTIME_REPORT_TRUST_BOUNDARY {
        findings.push(report_finding(
            report,
            "invalid_trust_boundary",
            "runtime input must retain the untrusted trust marker",
        ));
    }
    for (field, value) in [
        ("report_id", report.report_id.as_str()),
        ("runtime_graph_id", report.runtime_graph_id.as_str()),
        ("node_id", report.node_id.as_str()),
        ("attempt_id", report.attempt_id.as_str()),
        ("round_id", report.round_id.as_str()),
        (
            "expected_output_schema_id",
            report.expected_output_schema_id.as_str(),
        ),
        ("started_at", report.started_at.as_str()),
        ("finished_at", report.finished_at.as_str()),
    ] {
        if value.is_empty() {
            findings.push(report_finding(
                report,
                "empty_required_field",
                &format!("{field} must not be empty"),
            ));
        }
    }
    if !is_sha256(&report.runtime_graph_content_hash) {
        findings.push(report_finding(
            report,
            "invalid_graph_hash",
            "runtime_graph_content_hash must be 64 lowercase hexadecimal characters",
        ));
    }
    let started_at = parse_canonical_utc_timestamp(&report.started_at);
    let finished_at = parse_canonical_utc_timestamp(&report.finished_at);
    if started_at.is_none() {
        findings.push(report_finding(
            report,
            "invalid_started_at",
            "started_at must be a valid canonical UTC timestamp such as 2026-08-03T00:00:00Z",
        ));
    }
    if finished_at.is_none() {
        findings.push(report_finding(
            report,
            "invalid_finished_at",
            "finished_at must be a valid canonical UTC timestamp such as 2026-08-03T00:00:00Z",
        ));
    }
    if started_at
        .zip(finished_at)
        .is_some_and(|(start, finish)| finish < start)
    {
        findings.push(report_finding(
            report,
            "invalid_time_order",
            "finished_at must not precede started_at",
        ));
    }
    if report.retry_of_attempt_id.as_deref() == Some(report.attempt_id.as_str()) {
        findings.push(report_finding(
            report,
            "self_retry",
            "retry_of_attempt_id must name a different prior attempt",
        ));
    }
    for (field, value) in [
        ("retry_of_attempt_id", report.retry_of_attempt_id.as_deref()),
        ("reported_model", report.reported_model.as_deref()),
        ("reported_context_id", report.reported_context_id.as_deref()),
        ("worktree_id", report.worktree_id.as_deref()),
        ("commit_sha", report.commit_sha.as_deref()),
    ] {
        if value.is_some_and(str::is_empty) {
            findings.push(report_finding(
                report,
                "empty_optional_field",
                &format!("{field} must be null or non-empty"),
            ));
        }
    }
    for (field, values) in [
        ("parent_node_ids", report.parent_node_ids.as_slice()),
        ("input_artifact_ids", report.input_artifact_ids.as_slice()),
        ("output_artifact_ids", report.output_artifact_ids.as_slice()),
        ("verifier_report_ids", report.verifier_report_ids.as_slice()),
    ] {
        validate_id_list(report, field, values, &mut findings);
    }
    match (report.status, report.failure_kind) {
        (RuntimeNodeStatus::Succeeded, Some(_)) => findings.push(report_finding(
            report,
            "success_with_failure_kind",
            "a succeeded runtime status cannot carry failure_kind",
        )),
        (RuntimeNodeStatus::Succeeded, None) => {}
        (_, None) => findings.push(report_finding(
            report,
            "failure_kind_required",
            "a non-success runtime status must carry failure_kind",
        )),
        (_, Some(_)) => {}
    }
    if report
        .actual_output_schema_id
        .as_deref()
        .is_some_and(str::is_empty)
    {
        findings.push(report_finding(
            report,
            "empty_actual_output_schema",
            "actual_output_schema_id must be null or non-empty",
        ));
    }
    if let Some(usage) = &report.token_usage {
        if usage.input_tokens.saturating_add(usage.output_tokens) != usage.total_tokens {
            findings.push(report_finding(
                report,
                "token_total_mismatch",
                "total_tokens must equal input_tokens plus output_tokens",
            ));
        }
    }
    if report
        .cost
        .as_ref()
        .is_some_and(|cost| !cost.amount.is_finite() || cost.amount < 0.0)
    {
        findings.push(report_finding(
            report,
            "invalid_reported_cost",
            "reported cost must be finite and non-negative",
        ));
    }
    for (field, value) in [
        (
            "runtime_identity.runtime_name",
            report.runtime_identity.runtime_name.as_str(),
        ),
        (
            "runtime_identity.runtime_version",
            report.runtime_identity.runtime_version.as_str(),
        ),
        (
            "runtime_identity.adapter_name",
            report.runtime_identity.adapter_name.as_str(),
        ),
        (
            "runtime_identity.adapter_version",
            report.runtime_identity.adapter_version.as_str(),
        ),
    ] {
        if value.is_empty() {
            findings.push(report_finding(
                report,
                "empty_reported_runtime_identity",
                &format!("{field} must not be empty"),
            ));
        }
    }
    if report
        .cost
        .as_ref()
        .is_some_and(|cost| cost.currency.is_empty())
    {
        findings.push(report_finding(
            report,
            "empty_reported_cost_currency",
            "reported cost currency must not be empty",
        ));
    }
    for allocation in &report.resource_allocations {
        if allocation.resource_id.is_empty()
            || allocation.mode.is_empty()
            || allocation.allocation_id.is_empty()
        {
            findings.push(report_finding(
                report,
                "empty_reported_resource_allocation",
                "reported resource allocation fields must not be empty",
            ));
        }
    }
    findings
}

/// Produces the canonical JSON representation used for hashing and deduplication.
///
/// Struct field order and all sequence order are preserved. Callers must not
/// reinterpret runtime metadata as verified merely because it canonicalizes.
pub fn canonical_runtime_node_report(
    report: &RuntimeNodeReport,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(report)
}

/// Reconciles reports with an independently content-addressed topology.
///
/// `observed_artifact_ids` is the artifact inventory observed by the ingest
/// boundary. An artifact is unaccounted when no report declares it as an input
/// or output. Runtime success alone never makes `complete` true: graph joins,
/// retry lineage, output schemas, failures, missing reports, and artifact
/// accounting must all agree.
pub fn reconcile_runtime_reports(
    expected: &RuntimeGraphExpectation,
    reports: &[RuntimeNodeReport],
    observed_artifact_ids: &[String],
) -> RuntimeCompleteness {
    let mut findings = Vec::new();
    let mut expected_nodes = BTreeMap::new();
    for node in &expected.nodes {
        if expected_nodes
            .insert(
                node.node_id.as_str(),
                node.expected_output_schema_id.as_str(),
            )
            .is_some()
        {
            findings.push(finding(
                "duplicate_expected_node",
                Some(node.node_id.clone()),
                None,
                "topology expectation contains the node more than once",
            ));
        }
    }

    let mut attempt_indices = BTreeMap::new();
    let mut node_reports: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut accounted_artifacts = BTreeSet::new();
    let mut duplicate_attempt_indices = BTreeSet::new();
    for (index, report) in reports.iter().enumerate() {
        findings.extend(validate_runtime_node_report(report));
        if attempt_indices.contains_key(report.attempt_id.as_str()) {
            duplicate_attempt_indices.insert(index);
            findings.push(report_finding(
                report,
                "duplicate_attempt_id",
                "attempt_id appears more than once",
            ));
        } else {
            attempt_indices.insert(report.attempt_id.as_str(), index);
        }
        if report.runtime_graph_id != expected.runtime_graph_id
            || report.runtime_graph_content_hash != expected.runtime_graph_content_hash
        {
            findings.push(report_finding(
                report,
                "graph_join_mismatch",
                "runtime_graph_id and content hash must exactly match the topology expectation",
            ));
            continue;
        }
        if !expected_nodes.contains_key(report.node_id.as_str()) {
            findings.push(report_finding(
                report,
                "unexpected_node",
                "node_id does not exist in the topology expectation",
            ));
            continue;
        }
        for artifact in report
            .input_artifact_ids
            .iter()
            .chain(&report.output_artifact_ids)
        {
            accounted_artifacts.insert(artifact.as_str());
        }
        node_reports
            .entry(report.node_id.as_str())
            .or_default()
            .push(index);
    }

    let mut failed_nodes = BTreeSet::new();
    for (node_id, indices) in &node_reports {
        let mut roots = Vec::new();
        let mut children: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        let node_index_set = indices.iter().copied().collect::<BTreeSet<_>>();
        let mut valid_parent: BTreeMap<&str, &str> = BTreeMap::new();
        for &index in indices {
            let report = &reports[index];
            if let Some(prior_id) = report.retry_of_attempt_id.as_deref() {
                match attempt_indices.get(prior_id).copied() {
                    Some(prior_index)
                        if node_index_set.contains(&prior_index)
                            && prior_id != report.attempt_id
                            && reports[prior_index].status != RuntimeNodeStatus::Succeeded =>
                    {
                        children.entry(prior_id).or_default().push(index);
                        valid_parent.insert(report.attempt_id.as_str(), prior_id);
                    }
                    _ => {
                        duplicate_attempt_indices.insert(index);
                        findings.push(report_finding(
                            report,
                            "invalid_retry_lineage",
                            "retry_of_attempt_id must name an existing non-success attempt for the same graph node",
                        ));
                    }
                }
            } else {
                roots.push(index);
            }
        }
        if roots.len() > 1 {
            duplicate_attempt_indices.extend(roots.iter().skip(1).copied());
            findings.push(finding(
                "duplicate_attempt_without_lineage",
                Some((*node_id).to_owned()),
                None,
                "multiple root attempts exist for one node",
            ));
        }
        for (prior, child_indices) in children {
            if child_indices.len() > 1 {
                duplicate_attempt_indices.extend(child_indices.iter().skip(1).copied());
                findings.push(finding(
                    "branched_retry_lineage",
                    Some((*node_id).to_owned()),
                    Some(prior.to_owned()),
                    "one attempt has multiple retry successors",
                ));
            }
        }

        let mut cycle_attempts = BTreeSet::new();
        for attempt_id in valid_parent.keys().copied() {
            let mut seen = BTreeSet::new();
            let mut cursor = Some(attempt_id);
            while let Some(current) = cursor {
                if !seen.insert(current) {
                    cycle_attempts.extend(seen);
                    break;
                }
                cursor = valid_parent.get(current).copied();
            }
        }
        if !cycle_attempts.is_empty() {
            for attempt_id in &cycle_attempts {
                if let Some(index) = attempt_indices.get(attempt_id).copied() {
                    duplicate_attempt_indices.insert(index);
                }
            }
            findings.push(finding(
                "retry_lineage_cycle",
                Some((*node_id).to_owned()),
                None,
                format!("retry lineage contains a cycle involving {cycle_attempts:?}"),
            ));
        }

        let attempts_with_successors = valid_parent.values().copied().collect::<BTreeSet<_>>();
        let successful_schema_match = indices
            .iter()
            .filter(|&&index| {
                !attempts_with_successors.contains(reports[index].attempt_id.as_str())
            })
            .any(|&index| {
                let report = &reports[index];
                let expected_schema = expected_nodes[node_id];
                let schema_matches = report.expected_output_schema_id == expected_schema
                    && report.actual_output_schema_id.as_deref() == Some(expected_schema);
                if report.status == RuntimeNodeStatus::Succeeded && !schema_matches {
                    findings.push(report_finding(
                        report,
                        "output_schema_mismatch",
                        "runtime success does not hide an expected/actual output schema mismatch",
                    ));
                }
                report.status == RuntimeNodeStatus::Succeeded && schema_matches
            });
        if !successful_schema_match {
            failed_nodes.insert(*node_id);
        }
    }

    let missing_nodes = expected_nodes
        .keys()
        .filter(|node_id| !node_reports.contains_key(**node_id))
        .copied()
        .collect::<Vec<_>>();
    for node_id in &missing_nodes {
        findings.push(finding(
            "missing_report",
            Some((*node_id).to_owned()),
            None,
            "expected node has no graph-matching runtime report",
        ));
    }

    let unaccounted = observed_artifact_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .difference(&accounted_artifacts)
        .copied()
        .collect::<Vec<_>>();
    for artifact_id in &unaccounted {
        findings.push(finding(
            "unaccounted_artifact",
            None,
            None,
            format!("observed artifact {artifact_id:?} is not named by any report"),
        ));
    }

    findings.sort_by(|left, right| {
        (&left.code, &left.node_id, &left.attempt_id, &left.detail).cmp(&(
            &right.code,
            &right.node_id,
            &right.attempt_id,
            &right.detail,
        ))
    });
    let expected_node_count = expected_nodes.len() as u64;
    let missing_report_count = missing_nodes.len() as u64;
    let failed_node_count = failed_nodes.len() as u64;
    let unaccounted_artifact_count = unaccounted.len() as u64;
    let duplicate_attempt_count = duplicate_attempt_indices.len() as u64;
    let complete = expected_node_count > 0
        && findings.is_empty()
        && missing_report_count == 0
        && failed_node_count == 0
        && duplicate_attempt_count == 0
        && unaccounted_artifact_count == 0;

    RuntimeCompleteness {
        expected_node_count,
        actual_report_count: reports.len() as u64,
        failed_node_count,
        missing_report_count,
        duplicate_attempt_count,
        unaccounted_artifact_count,
        complete,
        findings,
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Parses the deliberately narrow timestamp representation used at the runtime boundary.
///
/// Requiring UTC and fixed-width calendar/time fields makes chronological comparison
/// deterministic without treating a runtime's clock as an independently trusted anchor.
fn parse_canonical_utc_timestamp(value: &str) -> Option<(u16, u8, u8, u8, u8, u8, u32)> {
    let (whole, fraction) = value.strip_suffix('Z')?.split_once('.').map_or_else(
        || (value.strip_suffix('Z').unwrap_or_default(), None),
        |(whole, fraction)| (whole, Some(fraction)),
    );
    let bytes = whole.as_bytes();
    if bytes.len() != 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let parse = |start: usize, end: usize| whole.get(start..end)?.parse::<u32>().ok();
    let year: u16 = parse(0, 4)?.try_into().ok()?;
    let month: u8 = parse(5, 7)?.try_into().ok()?;
    let day: u8 = parse(8, 10)?.try_into().ok()?;
    let hour: u8 = parse(11, 13)?.try_into().ok()?;
    let minute: u8 = parse(14, 16)?.try_into().ok()?;
    let second: u8 = parse(17, 19)?.try_into().ok()?;
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    #[allow(clippy::manual_is_multiple_of)]
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day == 0 || day > days {
        return None;
    }
    let nanos = match fraction {
        None => 0,
        Some(digits)
            if !digits.is_empty()
                && digits.len() <= 9
                && digits.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            let parsed: u32 = digits.parse().ok()?;
            parsed.checked_mul(10_u32.pow((9 - digits.len()) as u32))?
        }
        Some(_) => return None,
    };
    Some((year, month, day, hour, minute, second, nanos))
}

fn validate_id_list(
    report: &RuntimeNodeReport,
    field: &str,
    values: &[String],
    findings: &mut Vec<RuntimeProtocolFinding>,
) {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.is_empty() {
            findings.push(report_finding(
                report,
                "empty_list_id",
                &format!("{field} contains an empty identifier"),
            ));
        }
        if !seen.insert(value) {
            findings.push(report_finding(
                report,
                "duplicate_list_id",
                &format!("{field} contains duplicate identifier {value:?}"),
            ));
        }
    }
}

fn report_finding(report: &RuntimeNodeReport, code: &str, detail: &str) -> RuntimeProtocolFinding {
    finding(
        code,
        Some(report.node_id.clone()),
        Some(report.attempt_id.clone()),
        detail,
    )
}

fn finding(
    code: &str,
    node_id: Option<String>,
    attempt_id: Option<String>,
    detail: impl Into<String>,
) -> RuntimeProtocolFinding {
    RuntimeProtocolFinding {
        code: code.to_owned(),
        node_id,
        attempt_id,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn report(node: &str, attempt: &str) -> RuntimeNodeReport {
        RuntimeNodeReport {
            schema: RUNTIME_NODE_REPORT_SCHEMA.to_owned(),
            schema_version: 0,
            report_id: format!("report:{attempt}"),
            runtime_graph_id: "runtime_graph:example".to_owned(),
            runtime_graph_content_hash: HASH.to_owned(),
            node_id: node.to_owned(),
            attempt_id: attempt.to_owned(),
            retry_of_attempt_id: None,
            round_id: "round:1".to_owned(),
            parent_node_ids: Vec::new(),
            input_artifact_ids: Vec::new(),
            output_artifact_ids: vec![format!("artifact:{node}")],
            expected_output_schema_id: "schema:result".to_owned(),
            actual_output_schema_id: Some("schema:result".to_owned()),
            started_at: "2026-08-03T00:00:00Z".to_owned(),
            finished_at: "2026-08-03T00:00:01Z".to_owned(),
            status: RuntimeNodeStatus::Succeeded,
            failure_kind: None,
            runtime_identity: ReportedRuntimeIdentity {
                runtime_name: "fixture".to_owned(),
                runtime_version: "1".to_owned(),
                adapter_name: "jsonl".to_owned(),
                adapter_version: "1".to_owned(),
            },
            reported_model: Some("runtime-claimed-model".to_owned()),
            reported_context_id: Some("runtime-claimed-context".to_owned()),
            token_usage: Some(ReportedTokenUsage {
                input_tokens: 2,
                output_tokens: 3,
                total_tokens: 5,
            }),
            cost: Some(ReportedCost {
                amount: 0.01,
                currency: "USD".to_owned(),
            }),
            resource_allocations: Vec::new(),
            worktree_id: None,
            commit_sha: None,
            verifier_report_ids: Vec::new(),
            trust_boundary: RUNTIME_REPORT_TRUST_BOUNDARY.to_owned(),
        }
    }

    fn expectation(count: usize) -> RuntimeGraphExpectation {
        RuntimeGraphExpectation {
            runtime_graph_id: "runtime_graph:example".to_owned(),
            runtime_graph_content_hash: HASH.to_owned(),
            nodes: (0..count)
                .map(|index| ExpectedRuntimeNode {
                    node_id: format!("node:{index}"),
                    expected_output_schema_id: "schema:result".to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn example_parses_validates_and_canonicalizes_stably() {
        let source = include_str!("../schemas/experimental/runtime.node_report.example.json");
        let parsed = parse_runtime_node_report(source).expect("valid example");
        let canonical = canonical_runtime_node_report(&parsed).expect("canonical JSON");
        let reparsed = parse_runtime_node_report(&canonical).expect("canonical example");
        assert_eq!(parsed, reparsed);
        assert_eq!(canonical, canonical_runtime_node_report(&reparsed).unwrap());
    }

    #[test]
    fn reports_from_199_of_200_nodes_are_not_complete() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/runtime/199-of-200.json"))
                .unwrap();
        let expected_count = fixture["expected_node_count"].as_u64().unwrap() as usize;
        let report_count = fixture["reported_node_count"].as_u64().unwrap() as usize;
        let reports = (0..report_count)
            .map(|index| report(&format!("node:{index}"), &format!("attempt:{index}")))
            .collect::<Vec<_>>();
        let result = reconcile_runtime_reports(&expectation(expected_count), &reports, &[]);
        assert!(!result.complete);
        assert_eq!(result.expected_node_count, 200);
        assert_eq!(result.actual_report_count, 199);
        assert_eq!(result.missing_report_count, 1);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.code == "missing_report"
                && finding.node_id.as_deref() == Some("node:199")));
    }

    #[test]
    fn duplicate_root_attempts_fail_but_an_exact_linear_retry_is_reconciled() {
        let expected = expectation(1);
        let first = report("node:0", "attempt:1");
        let mut duplicate = report("node:0", "attempt:2");
        let result = reconcile_runtime_reports(&expected, &[first.clone(), duplicate.clone()], &[]);
        assert!(!result.complete);
        assert_eq!(result.duplicate_attempt_count, 1);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate_attempt_without_lineage"));

        duplicate.retry_of_attempt_id = Some(first.attempt_id.clone());
        let mut failed_first = first;
        failed_first.status = RuntimeNodeStatus::Failed;
        failed_first.failure_kind = Some(RuntimeFailureKind::ExecutionError);
        let result = reconcile_runtime_reports(&expected, &[failed_first, duplicate], &[]);
        assert!(
            result.complete,
            "an explicit linear retry may complete the node"
        );
        assert_eq!(result.duplicate_attempt_count, 0);
        assert!(!result
            .findings
            .iter()
            .any(|finding| finding.code == "invalid_retry_lineage"));
    }

    #[test]
    fn success_cannot_hide_schema_mismatch_or_unaccounted_artifact() {
        let mut runtime_report = report("node:0", "attempt:1");
        runtime_report.actual_output_schema_id = Some("schema:wrong".to_owned());
        let result = reconcile_runtime_reports(
            &expectation(1),
            &[runtime_report],
            &["artifact:orphan".to_owned()],
        );
        assert!(!result.complete);
        assert_eq!(result.failed_node_count, 1);
        assert_eq!(result.unaccounted_artifact_count, 1);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.code == "output_schema_mismatch"));
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.code == "unaccounted_artifact"));
    }

    #[test]
    fn reported_model_and_context_never_change_completeness() {
        let expected = expectation(1);
        let mut first = report("node:0", "attempt:1");
        let first_result = reconcile_runtime_reports(&expected, &[first.clone()], &[]);
        first.reported_model = Some("different-untrusted-claim".to_owned());
        first.reported_context_id = None;
        assert_eq!(
            first_result,
            reconcile_runtime_reports(&expected, &[first], &[])
        );
        assert!(first_result.complete);
    }

    #[test]
    fn graph_hash_and_node_id_are_both_required_for_the_join() {
        let expected = expectation(1);
        for mutator in [
            |report: &mut RuntimeNodeReport| report.runtime_graph_content_hash = "f".repeat(64),
            |report: &mut RuntimeNodeReport| report.node_id = "node:unexpected".to_owned(),
        ] {
            let mut candidate = report("node:0", "attempt:1");
            mutator(&mut candidate);
            let result = reconcile_runtime_reports(&expected, &[candidate], &[]);
            assert!(!result.complete);
            assert_eq!(result.missing_report_count, 1);
        }
    }

    #[test]
    fn retry_lineage_is_explicit_and_not_inferred_from_report_order() {
        let expected = expectation(1);
        let mut prior = report("node:0", "attempt:prior");
        prior.status = RuntimeNodeStatus::TimedOut;
        prior.failure_kind = Some(RuntimeFailureKind::Timeout);
        let mut retry = report("node:0", "attempt:retry");
        retry.retry_of_attempt_id = Some(prior.attempt_id.clone());

        let result = reconcile_runtime_reports(&expected, &[retry, prior], &[]);
        assert!(result.complete);
        assert_eq!(result.duplicate_attempt_count, 0);
    }

    #[test]
    fn one_duplicate_attempt_is_counted_once_even_when_it_breaks_multiple_rules() {
        let expected = expectation(1);
        let first = report("node:0", "attempt:same");
        let second = first.clone();
        let result = reconcile_runtime_reports(&expected, &[first, second], &[]);
        assert!(!result.complete);
        assert_eq!(result.duplicate_attempt_count, 1);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.code == "duplicate_attempt_id"));
    }

    #[test]
    fn a_report_outside_the_graph_cannot_account_for_an_observed_artifact() {
        let expected = expectation(1);
        let matched = report("node:0", "attempt:matched");
        let mut outside = report("node:outside", "attempt:outside");
        outside.output_artifact_ids = vec!["artifact:orphan".to_owned()];
        let result = reconcile_runtime_reports(
            &expected,
            &[matched, outside],
            &["artifact:orphan".to_owned()],
        );
        assert!(!result.complete);
        assert_eq!(result.unaccounted_artifact_count, 1);
    }

    #[test]
    fn timestamps_are_calendar_valid_and_chronologically_ordered() {
        let mut candidate = report("node:0", "attempt:time");
        candidate.started_at = "2026-02-29T00:00:00Z".to_owned();
        candidate.finished_at = "not-a-timestamp".to_owned();
        let findings = validate_runtime_node_report(&candidate);
        assert!(findings
            .iter()
            .any(|finding| finding.code == "invalid_started_at"));
        assert!(findings
            .iter()
            .any(|finding| finding.code == "invalid_finished_at"));

        candidate.started_at = "2026-08-03T00:00:01.000000001Z".to_owned();
        candidate.finished_at = "2026-08-03T00:00:01Z".to_owned();
        let findings = validate_runtime_node_report(&candidate);
        assert!(findings
            .iter()
            .any(|finding| finding.code == "invalid_time_order"));

        candidate.started_at = "2024-02-29T00:00:00.1Z".to_owned();
        candidate.finished_at = "2024-02-29T00:00:00.10Z".to_owned();
        assert!(validate_runtime_node_report(&candidate).is_empty());
    }
}

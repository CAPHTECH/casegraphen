//! Deterministic reconciliation for an external streaming runtime.
//!
//! Transport and dispatch remain runtime concerns. This module only orders
//! observations, derives unaccepted early-release proposals, and delegates
//! terminal completeness to the runtime protocol.

use crate::{
    execution_topology::{
        execution_topology_content_hash, DeliveryMode, EdgeKind, ExecutionTopology,
    },
    native_eval::evaluate_native_case,
    native_hash::sha256_hex,
    native_model::CaseSpace,
    resource_protocol::validate_resource_declaration,
    runtime_integration::{RuntimeIntegrationReport, RuntimeResourceExpectation},
    runtime_protocol::{
        reconcile_runtime_reports, RuntimeCompleteness, RuntimeGraphExpectation, RuntimeNodeReport,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const STREAM_EVENT_SCHEMA: &str = "casegraphen.experimental.runtime.stream_event.v0";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StreamEventPayload {
    ArtifactChunk {
        edge_id: String,
        artifact_id: String,
        schema_id: String,
        chunk_index: u64,
        chunk_sha256: String,
        final_chunk: bool,
    },
    NodeTerminal {
        status: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeStreamEvent {
    pub schema: String,
    pub event_id: String,
    pub runtime_graph_id: String,
    pub runtime_graph_content_hash: String,
    pub node_id: String,
    pub attempt_id: String,
    pub sequence: u64,
    pub logical_order: u64,
    pub observed_at: String,
    #[serde(flatten)]
    pub payload: StreamEventPayload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamRunStatus {
    Collecting,
    PartiallyProgressing,
    Complete,
    IncompleteTerminal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StreamFinding {
    pub code: String,
    pub event_id: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EarlyReleaseProposal {
    pub edge_id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub artifact_id: String,
    pub target_attempt_id: String,
    pub topology_content_hash: String,
    pub case_revision_id: String,
    pub resource_reconciliation_hash: String,
    pub source_event_id: String,
    pub accepted: bool,
}

/// Opaque projection of topology-bound, canonically reconciled resource
/// reservations. Callers cannot associate an arbitrary reconciliation with a
/// downstream node.
#[derive(Debug)]
pub struct StreamingResourcePermits {
    permits_by_target: BTreeMap<String, StreamingResourcePermit>,
}

#[derive(Debug)]
struct StreamingResourcePermit {
    topology_content_hash: String,
    case_revision_id: String,
    target_node_id: String,
    target_attempt_id: String,
    resource_reconciliation_hash: String,
}

/// Derives streaming permits from the exact topology-bound resource
/// expectations and the resource reconciliations emitted by the generic
/// runtime integrator.
pub fn derive_streaming_resource_permits(
    topology: &ExecutionTopology,
    expectations: &[RuntimeResourceExpectation],
    integration: &RuntimeIntegrationReport,
    acceptance: &StreamingAcceptance,
) -> Result<StreamingResourcePermits, Vec<StreamFinding>> {
    let topology_hash = execution_topology_content_hash(topology)
        .expect("typed execution topology serializes deterministically");
    let mut findings = Vec::new();
    if integration.topology_id != topology.topology_id
        || integration.topology_content_hash != topology_hash
    {
        findings.push(finding(
            "resource_permit_graph_mismatch",
            None,
            "resource integration result must join the exact streaming topology",
        ));
    }
    if acceptance.topology_content_hash != topology_hash || acceptance.case_revision_id.is_empty() {
        findings.push(finding(
            "resource_permit_acceptance_mismatch",
            None,
            "resource permits require canonical readiness for the exact topology and case revision",
        ));
    }
    if integration
        .ingest_findings
        .iter()
        .any(|finding| finding.code.contains("resource"))
    {
        findings.push(finding(
            "resource_integration_has_findings",
            None,
            "resource integration findings prevent streaming permits",
        ));
    }
    let mut permits_by_target = BTreeMap::new();
    let mut matched_reconciliations = BTreeSet::new();
    for expectation in expectations {
        for resource_finding in validate_resource_declaration(topology, &expectation.declaration) {
            findings.push(finding(
                "resource_permit_declaration_mismatch",
                None,
                format!("{}: {}", resource_finding.code, resource_finding.detail),
            ));
        }
        if permits_by_target.contains_key(&expectation.declaration.node_id) {
            findings.push(finding(
                "duplicate_resource_permit_target",
                None,
                format!(
                    "{} has more than one resource expectation",
                    expectation.declaration.node_id
                ),
            ));
        }
        let matches = integration
            .resource_reconciliations
            .iter()
            .enumerate()
            .filter(|(_, reconciliation)| {
                reconciliation.declaration_id == expectation.declaration.declaration_id
                    && reconciliation.reservation_id == expectation.reservation.reservation_id
                    && reconciliation.attempt_id == expectation.reservation.attempt_id
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            findings.push(finding(
                "resource_permit_reconciliation_join_mismatch",
                None,
                format!(
                    "{}/{} requires exactly one canonical reconciliation",
                    expectation.declaration.node_id, expectation.reservation.attempt_id
                ),
            ));
            continue;
        }
        let (index, reconciliation) = matches[0];
        matched_reconciliations.insert(index);
        let canonical = serde_json::to_vec(reconciliation)
            .expect("typed resource reconciliation serializes deterministically");
        let reconciliation_hash = sha256_hex(&canonical);
        if !integration.has_canonical_resource_binding(
            &expectation.declaration.node_id,
            &expectation.declaration.declaration_id,
            &expectation.reservation.reservation_id,
            &expectation.reservation.attempt_id,
            &reconciliation_hash,
        ) {
            findings.push(finding(
                "resource_permit_missing_integration_provenance",
                None,
                format!(
                    "{}/{} was not reconciled from this topology-bound expectation",
                    expectation.declaration.node_id, expectation.reservation.attempt_id
                ),
            ));
        }
        if !reconciliation.complete || !reconciliation.findings.is_empty() {
            findings.push(finding(
                "resource_permit_reconciliation_incomplete",
                None,
                format!(
                    "{}/{} resource reconciliation is incomplete",
                    expectation.declaration.node_id, expectation.reservation.attempt_id
                ),
            ));
        } else {
            permits_by_target.insert(
                expectation.declaration.node_id.clone(),
                StreamingResourcePermit {
                    topology_content_hash: topology_hash.clone(),
                    case_revision_id: acceptance.case_revision_id.clone(),
                    target_node_id: expectation.declaration.node_id.clone(),
                    target_attempt_id: expectation.reservation.attempt_id.clone(),
                    resource_reconciliation_hash: reconciliation_hash,
                },
            );
        }
    }
    if matched_reconciliations.len() != integration.resource_reconciliations.len() {
        findings.push(finding(
            "unmatched_resource_reconciliation",
            None,
            "integration result contains a reconciliation outside the supplied topology expectations",
        ));
    }
    findings.sort_by(|left, right| {
        (&left.code, &left.event_id, &left.detail).cmp(&(
            &right.code,
            &right.event_id,
            &right.detail,
        ))
    });
    if findings.is_empty() {
        Ok(StreamingResourcePermits { permits_by_target })
    } else {
        Err(findings)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamingReconciliation {
    pub status: StreamRunStatus,
    pub logical_events: Vec<RuntimeStreamEvent>,
    pub duplicate_event_count: u64,
    pub early_release_proposals: Vec<EarlyReleaseProposal>,
    pub unfinished_node_ids: Vec<String>,
    pub final_completeness: RuntimeCompleteness,
    pub findings: Vec<StreamFinding>,
}

/// Opaque projection of CaseGraphen's canonical readiness decision for one
/// topology revision. Callers cannot manufacture ready work-cell ids.
pub struct StreamingAcceptance {
    topology_content_hash: String,
    case_revision_id: String,
    ready_work_cell_ids: BTreeSet<String>,
}

pub fn derive_streaming_acceptance(
    case_space: &CaseSpace,
    topology: &ExecutionTopology,
) -> Result<StreamingAcceptance, StreamFinding> {
    if case_space.case_space_id.to_string() != topology.case_space_id {
        return Err(finding(
            "case_topology_identity_mismatch",
            None,
            "case space id must match the topology acceptance boundary",
        ));
    }
    let evaluation = evaluate_native_case(case_space).map_err(|error| {
        finding(
            "case_evaluation_refused",
            None,
            format!("canonical CaseGraphen evaluation refused: {error:?}"),
        )
    })?;
    Ok(StreamingAcceptance {
        topology_content_hash: execution_topology_content_hash(topology)
            .expect("typed topology serializes"),
        case_revision_id: case_space.revision.revision_id.to_string(),
        ready_work_cell_ids: evaluation
            .readiness
            .ready_cell_ids
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
    })
}

pub struct StreamingReconciliationInput<'a> {
    pub topology: &'a ExecutionTopology,
    pub expectation: &'a RuntimeGraphExpectation,
    pub events: &'a [RuntimeStreamEvent],
    pub terminal_reports: &'a [RuntimeNodeReport],
    pub observed_artifact_ids: &'a [String],
    /// Exact case revision for which this stream prefix is being reconciled.
    /// It must join both canonical readiness and every resource permit.
    pub expected_case_revision_id: &'a str,
    /// Opaque projection derived from topology-bound canonical resource
    /// reconciliation. Required for every early release.
    pub resource_permits: Option<&'a StreamingResourcePermits>,
    /// Canonical CaseGraphen readiness projection. Required only when a target
    /// has evidence/review/authority edges.
    pub acceptance: Option<&'a StreamingAcceptance>,
    pub run_closed: bool,
}

pub fn reconcile_stream(input: StreamingReconciliationInput<'_>) -> StreamingReconciliation {
    let mut findings = Vec::new();
    if input.expected_case_revision_id.is_empty() {
        findings.push(finding(
            "empty_expected_case_revision",
            None,
            "stream reconciliation requires an exact non-empty case revision",
        ));
    }
    if let Some(acceptance) = input.acceptance {
        if acceptance.case_revision_id != input.expected_case_revision_id {
            findings.push(finding(
                "stale_streaming_acceptance",
                None,
                "canonical readiness was derived for a different case revision",
            ));
        }
    }
    let topology_join_valid = input.topology.topology_id == input.expectation.runtime_graph_id
        && execution_topology_content_hash(input.topology)
            .is_ok_and(|hash| hash == input.expectation.runtime_graph_content_hash);
    if !topology_join_valid {
        findings.push(finding(
            "topology_expectation_mismatch",
            None,
            "topology identity/content hash must match the runtime expectation",
        ));
    }
    let expected_node_ids = input
        .expectation
        .nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut unique = BTreeMap::new();
    let mut duplicate_event_count = 0;
    for event in input.events {
        if event.schema != STREAM_EVENT_SCHEMA
            || event.event_id.is_empty()
            || event.node_id.is_empty()
            || event.attempt_id.is_empty()
        {
            findings.push(finding(
                "invalid_event_identity",
                Some(event.event_id.clone()),
                "schema and stable identities must be present",
            ));
            continue;
        }
        if event.runtime_graph_id != input.expectation.runtime_graph_id
            || event.runtime_graph_content_hash != input.expectation.runtime_graph_content_hash
        {
            findings.push(finding(
                "stream_graph_join_mismatch",
                Some(event.event_id.clone()),
                "event does not join the accepted deployment graph",
            ));
            continue;
        }
        if !expected_node_ids.contains(event.node_id.as_str()) {
            findings.push(finding(
                "unknown_stream_node",
                Some(event.event_id.clone()),
                "event node does not exist in the runtime graph expectation",
            ));
            continue;
        }
        match unique.get(event.event_id.as_str()) {
            Some(existing) if existing == event => duplicate_event_count += 1,
            Some(_) => findings.push(finding(
                "event_identity_collision",
                Some(event.event_id.clone()),
                "one event id names different bytes",
            )),
            None => {
                unique.insert(event.event_id.as_str(), event.clone());
            }
        }
    }
    let mut logical_events = unique.into_values().collect::<Vec<_>>();
    logical_events.sort_by(|a, b| {
        (
            a.logical_order,
            &a.node_id,
            &a.attempt_id,
            a.sequence,
            &a.event_id,
        )
            .cmp(&(
                b.logical_order,
                &b.node_id,
                &b.attempt_id,
                b.sequence,
                &b.event_id,
            ))
    });
    for ((node, attempt), sequences) in sequence_groups(&logical_events) {
        if let Some((expected, actual)) = sequences
            .iter()
            .enumerate()
            .find(|(i, sequence)| **sequence != *i as u64)
        {
            findings.push(finding(
                "non_contiguous_attempt_sequence",
                None,
                format!("{node}/{attempt} expected {expected}, observed {actual}"),
            ));
        }
    }
    for ((node, attempt, sequence), event_ids) in sequence_identity_groups(&logical_events) {
        if event_ids.len() > 1 {
            findings.push(finding(
                "attempt_sequence_collision",
                None,
                format!("{node}/{attempt} sequence {sequence} is claimed by events {event_ids:?}"),
            ));
        }
    }
    validate_chunk_sequences(&logical_events, &mut findings);
    // A gap or equivocation means the canonical stream prefix is not yet
    // established. Do not release a different event while that ambiguity is
    // unresolved; terminal completeness remains independently delegated.
    let stream_prefix_valid = findings.is_empty();

    let nodes = input
        .topology
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let edges = input
        .topology
        .edges
        .iter()
        .map(|edge| (edge.edge_id.as_str(), edge))
        .collect::<BTreeMap<_, _>>();
    let mut releases = Vec::new();
    for event in &logical_events {
        let StreamEventPayload::ArtifactChunk {
            edge_id,
            artifact_id,
            schema_id,
            chunk_sha256,
            ..
        } = &event.payload
        else {
            continue;
        };
        if !is_sha256(chunk_sha256) {
            findings.push(finding(
                "invalid_chunk_hash",
                Some(event.event_id.clone()),
                "chunk hash must be lowercase SHA-256",
            ));
            continue;
        }
        let Some(edge) = edges.get(edge_id.as_str()) else {
            findings.push(finding(
                "unknown_stream_edge",
                Some(event.event_id.clone()),
                "chunk names an edge outside the topology",
            ));
            continue;
        };
        if edge.from != event.node_id
            || edge.kind != EdgeKind::Data
            || edge.schema_id.as_deref() != Some(schema_id)
        {
            findings.push(finding(
                "stream_edge_contract_mismatch",
                Some(event.event_id.clone()),
                "producer, edge kind, or schema differs from topology",
            ));
            continue;
        }
        let streams = nodes
            .get(edge.from.as_str())
            .is_some_and(|node| node.delivery == DeliveryMode::Streaming);
        let resource_permit = input.resource_permits.and_then(|permits| {
            permits.permits_by_target.get(&edge.to).filter(|permit| {
                permit.topology_content_hash == input.expectation.runtime_graph_content_hash
                    && permit.case_revision_id == input.expected_case_revision_id
                    && permit.target_node_id == edge.to
            })
        });
        let resources = resource_permit.is_some();
        let has_acceptance_gate = input.topology.edges.iter().any(|candidate| {
            candidate.to == edge.to
                && matches!(
                    candidate.kind,
                    EdgeKind::Evidence | EdgeKind::ReviewOrAuthority
                )
        });
        let acceptance_satisfied = !has_acceptance_gate
            || input.acceptance.is_some_and(|acceptance| {
                acceptance.topology_content_hash == input.expectation.runtime_graph_content_hash
                    && acceptance.case_revision_id == input.expected_case_revision_id
                    && nodes.get(edge.to.as_str()).is_some_and(|target| {
                        acceptance
                            .ready_work_cell_ids
                            .contains(&target.work_cell_id)
                    })
            });
        if topology_join_valid
            && stream_prefix_valid
            && streams
            && resources
            && acceptance_satisfied
        {
            releases.push(EarlyReleaseProposal {
                edge_id: edge.edge_id.clone(),
                from_node_id: edge.from.clone(),
                to_node_id: edge.to.clone(),
                artifact_id: artifact_id.clone(),
                target_attempt_id: resource_permit
                    .expect("resource condition established target permit")
                    .target_attempt_id
                    .clone(),
                topology_content_hash: input.expectation.runtime_graph_content_hash.clone(),
                case_revision_id: input.expected_case_revision_id.to_owned(),
                resource_reconciliation_hash: resource_permit
                    .expect("resource condition established target permit")
                    .resource_reconciliation_hash
                    .clone(),
                source_event_id: event.event_id.clone(),
                accepted: false,
            });
        } else {
            findings.push(finding(
                "early_release_blocked",
                Some(event.event_id.clone()),
                "streaming, resource, or acceptance contract blocks release",
            ));
        }
    }
    releases.sort_by(|a, b| {
        (&a.edge_id, &a.artifact_id, &a.source_event_id).cmp(&(
            &b.edge_id,
            &b.artifact_id,
            &b.source_event_id,
        ))
    });
    let final_completeness = reconcile_runtime_reports(
        input.expectation,
        input.terminal_reports,
        input.observed_artifact_ids,
    );
    let reported = input
        .terminal_reports
        .iter()
        .filter(|report| {
            report.runtime_graph_id == input.expectation.runtime_graph_id
                && report.runtime_graph_content_hash == input.expectation.runtime_graph_content_hash
        })
        .map(|report| report.node_id.as_str())
        .collect::<BTreeSet<_>>();
    let unfinished_node_ids = input
        .expectation
        .nodes
        .iter()
        .filter(|node| !reported.contains(node.node_id.as_str()))
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>();
    let status = if final_completeness.complete {
        StreamRunStatus::Complete
    } else if input.run_closed {
        StreamRunStatus::IncompleteTerminal
    } else if releases.is_empty() {
        StreamRunStatus::Collecting
    } else {
        StreamRunStatus::PartiallyProgressing
    };
    findings
        .sort_by(|a, b| (&a.code, &a.event_id, &a.detail).cmp(&(&b.code, &b.event_id, &b.detail)));
    StreamingReconciliation {
        status,
        logical_events,
        duplicate_event_count,
        early_release_proposals: releases,
        unfinished_node_ids,
        final_completeness,
        findings,
    }
}

type AttemptSequences<'a> = BTreeMap<(&'a str, &'a str), Vec<u64>>;

fn sequence_groups(events: &[RuntimeStreamEvent]) -> AttemptSequences<'_> {
    let mut groups: AttemptSequences<'_> = BTreeMap::new();
    for event in events {
        groups
            .entry((event.node_id.as_str(), event.attempt_id.as_str()))
            .or_default()
            .push(event.sequence);
    }
    for values in groups.values_mut() {
        values.sort_unstable();
        values.dedup();
    }
    groups
}

fn sequence_identity_groups(
    events: &[RuntimeStreamEvent],
) -> BTreeMap<(&str, &str, u64), Vec<&str>> {
    let mut groups = BTreeMap::new();
    for event in events {
        groups
            .entry((
                event.node_id.as_str(),
                event.attempt_id.as_str(),
                event.sequence,
            ))
            .or_insert_with(Vec::new)
            .push(event.event_id.as_str());
    }
    groups
}

type ChunkStreamKey<'a> = (&'a str, &'a str, &'a str, &'a str);
type ChunkObservation<'a> = (u64, bool, &'a str);

fn validate_chunk_sequences(events: &[RuntimeStreamEvent], findings: &mut Vec<StreamFinding>) {
    let mut groups: BTreeMap<ChunkStreamKey<'_>, Vec<ChunkObservation<'_>>> = BTreeMap::new();
    for event in events {
        if let StreamEventPayload::ArtifactChunk {
            edge_id,
            artifact_id,
            chunk_index,
            final_chunk,
            ..
        } = &event.payload
        {
            groups
                .entry((
                    event.node_id.as_str(),
                    event.attempt_id.as_str(),
                    edge_id.as_str(),
                    artifact_id.as_str(),
                ))
                .or_default()
                .push((*chunk_index, *final_chunk, event.event_id.as_str()));
        }
    }
    for ((node, attempt, edge, artifact), mut chunks) in groups {
        chunks.sort_by_key(|(index, _, event)| (*index, *event));
        let indices = chunks
            .iter()
            .map(|(index, _, _)| *index)
            .collect::<Vec<_>>();
        let distinct = indices.iter().copied().collect::<BTreeSet<_>>();
        if distinct.len() != indices.len() {
            findings.push(finding(
                "chunk_index_collision",
                None,
                format!("{node}/{attempt}/{edge}/{artifact} repeats a chunk index"),
            ));
        }
        if distinct.iter().copied().ne(0..distinct.len() as u64) {
            findings.push(finding(
                "non_contiguous_chunk_sequence",
                None,
                format!("{node}/{attempt}/{edge}/{artifact} has chunk indices {distinct:?}"),
            ));
        }
        let finals = chunks
            .iter()
            .filter(|(_, final_chunk, _)| *final_chunk)
            .map(|(index, _, _)| *index)
            .collect::<Vec<_>>();
        if finals.len() > 1
            || finals
                .first()
                .is_some_and(|index| Some(index) != distinct.last())
        {
            findings.push(finding(
                "invalid_final_chunk_position",
                None,
                format!("{node}/{attempt}/{edge}/{artifact} final chunk indices are {finals:?}"),
            ));
        }
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn finding(code: &str, event_id: Option<String>, detail: impl Into<String>) -> StreamFinding {
    StreamFinding {
        code: code.into(),
        event_id,
        detail: detail.into(),
    }
}

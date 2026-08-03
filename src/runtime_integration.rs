//! Generic JSONL ingest and reconciliation at the untrusted runtime boundary.
//!
//! This adapter stores observations and emits reviewable proposals. It never
//! calls a runtime and never accepts evidence, morphisms, or runtime claims.

use crate::{
    execution_topology::{execution_topology_content_hash, ExecutionTopology},
    graph_lint::{lint_execution_topology, FindingClassification, LintSeverity},
    resource_protocol::{
        reconcile_resource_allocations, validate_resource_declaration, ResourceDeclaration,
        ResourceReconciliation, ResourceReservation, RuntimeResourceAllocation,
    },
    runtime_protocol::{
        canonical_runtime_node_report, parse_runtime_node_report, reconcile_runtime_reports,
        ExpectedRuntimeNode, RuntimeCompleteness, RuntimeGraphExpectation, RuntimeNodeReport,
        RuntimeNodeStatus,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const RUNTIME_INTEGRATION_REPORT_SCHEMA: &str =
    "casegraphen.experimental.runtime.integration_report.v0";
/// Schema identity of one generic JSONL ingest envelope.
pub const RUNTIME_INTEGRATION_RECORD_SCHEMA: &str =
    "casegraphen.experimental.runtime.integration.jsonl_record.v0";
pub const RESOURCE_EXPECTATION_BUNDLE_SCHEMA: &str =
    "casegraphen.experimental.runtime.resource_expectation_bundle.v0";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceExpectationBundle {
    pub schema: String,
    pub schema_version: u32,
    pub topology_content_hash: String,
    pub case_revision_id: String,
    pub expectations: Vec<ResourceExpectationBundleEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceExpectationBundleEntry {
    pub node_id: String,
    pub attempt_id: String,
    pub declaration: ResourceDeclaration,
    pub reservation: ResourceReservation,
    pub allocations: Vec<RuntimeResourceAllocation>,
    pub disposition_evidence: Vec<crate::resource_protocol::ReservationDispositionAssertion>,
}

impl ResourceExpectationBundle {
    pub fn validate(
        &self,
        topology: &ExecutionTopology,
        case_revision_id: &str,
    ) -> Result<Vec<RuntimeResourceExpectation>, Vec<IngestFinding>> {
        let mut findings = Vec::new();
        let topology_hash =
            execution_topology_content_hash(topology).expect("typed execution topology serializes");
        if self.schema != RESOURCE_EXPECTATION_BUNDLE_SCHEMA || self.schema_version != 0 {
            findings.push(bundle_finding(
                "unsupported_resource_expectation_bundle",
                "schema/version must name runtime.resource_expectation_bundle.v0",
            ));
        }
        if self.topology_content_hash != topology_hash {
            findings.push(bundle_finding(
                "resource_bundle_topology_mismatch",
                "bundle must name the exact topology content hash",
            ));
        }
        if self.case_revision_id != case_revision_id {
            findings.push(bundle_finding(
                "resource_bundle_revision_mismatch",
                "bundle must name the exact client-observed case revision",
            ));
        }
        let mut nodes = BTreeSet::new();
        let mut attempts = BTreeSet::new();
        let mut reservations = BTreeSet::new();
        let mut allocations = BTreeSet::new();
        for entry in &self.expectations {
            if !nodes.insert(entry.node_id.as_str()) {
                findings.push(bundle_finding(
                    "duplicate_resource_bundle_node",
                    &format!("{} appears more than once", entry.node_id),
                ));
            }
            if !attempts.insert(entry.attempt_id.as_str()) {
                findings.push(bundle_finding(
                    "duplicate_resource_bundle_attempt",
                    &format!("{} appears more than once", entry.attempt_id),
                ));
            }
            if !reservations.insert(entry.reservation.reservation_id.as_str()) {
                findings.push(bundle_finding(
                    "duplicate_resource_bundle_reservation",
                    &format!(
                        "{} appears more than once",
                        entry.reservation.reservation_id
                    ),
                ));
            }
            if entry.node_id != entry.declaration.node_id
                || entry.attempt_id != entry.reservation.attempt_id
                || entry.declaration.declaration_id != entry.reservation.declaration_id
            {
                findings.push(bundle_finding(
                    "resource_bundle_join_mismatch",
                    &format!("{} identities do not join", entry.node_id),
                ));
            }
            for allocation in &entry.allocations {
                if !allocations.insert(allocation.allocation_id.as_str()) {
                    findings.push(bundle_finding(
                        "duplicate_resource_bundle_allocation",
                        &format!("{} appears more than once", allocation.allocation_id),
                    ));
                }
                if allocation.reservation_id != entry.reservation.reservation_id
                    || allocation.attempt_id != entry.attempt_id
                {
                    findings.push(bundle_finding(
                        "resource_bundle_allocation_join_mismatch",
                        &format!(
                            "{} does not join {}",
                            allocation.allocation_id, entry.reservation.reservation_id
                        ),
                    ));
                }
            }
            for assertion in &entry.disposition_evidence {
                if assertion.reservation_id != entry.reservation.reservation_id
                    || assertion.attempt_id != entry.attempt_id
                {
                    findings.push(bundle_finding(
                        "resource_bundle_disposition_join_mismatch",
                        &format!(
                            "{} does not join {}",
                            assertion.assertion_id, entry.reservation.reservation_id
                        ),
                    ));
                }
            }
        }
        if findings.is_empty() {
            Ok(self
                .expectations
                .iter()
                .map(|entry| RuntimeResourceExpectation {
                    declaration: entry.declaration.clone(),
                    reservation: entry.reservation.clone(),
                })
                .collect())
        } else {
            findings.sort_by(|left, right| {
                (&left.code, &left.detail).cmp(&(&right.code, &right.detail))
            });
            Err(findings)
        }
    }

    pub fn allocation_jsonl(&self) -> String {
        self.expectations
            .iter()
            .flat_map(|entry| &entry.allocations)
            .map(|allocation| {
                serde_json::to_string(
                    &json!({"kind":"resource_allocation","allocation":allocation}),
                )
                .expect("typed allocation serializes")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn bundle_finding(code: &str, detail: &str) -> IngestFinding {
    IngestFinding {
        code: code.to_owned(),
        line: None,
        detail: detail.to_owned(),
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum JsonlRecord {
    NodeReport {
        report: Value,
    },
    Artifact {
        artifact_id: String,
        media_type: String,
        content: String,
    },
    ResourceAllocation {
        allocation: Value,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IngestFinding {
    pub code: String,
    pub line: Option<usize>,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContentAddressedArtifact {
    pub artifact_id: String,
    pub sha256: String,
    pub media_type: String,
    pub byte_length: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    Evidence,
    Morphism,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalReviewStatus {
    Unreviewed,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IntegrationProposal {
    pub proposal_id: String,
    pub kind: ProposalKind,
    pub review_status: ProposalReviewStatus,
    pub source_boundary: String,
    pub payload: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationHalt {
    IncompleteRuntimeReports,
    ResourceReconciliationIncomplete,
    NeedsReview,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RuntimeIntegrationReport {
    pub schema: &'static str,
    pub report_version: u32,
    pub topology_id: String,
    pub topology_content_hash: String,
    /// Preserved from the caller; never replaced with a current revision.
    pub base_revision_id: String,
    pub accepted: bool,
    /// Completeness of the whole integration boundary, not only node reports.
    pub reconciliation_complete: bool,
    pub halt: IntegrationHalt,
    pub completeness: RuntimeCompleteness,
    pub resource_reconciliations: Vec<ResourceReconciliation>,
    pub artifacts: Vec<ContentAddressedArtifact>,
    pub proposals: Vec<IntegrationProposal>,
    pub ingest_findings: Vec<IngestFinding>,
    /// Non-wire provenance proving which topology-bound expectations were
    /// actually passed through this reconciler invocation.
    #[serde(skip)]
    canonical_resource_bindings: BTreeSet<(String, String, String, String, String)>,
}

impl RuntimeIntegrationReport {
    pub(crate) fn has_canonical_resource_binding(
        &self,
        node_id: &str,
        declaration_id: &str,
        reservation_id: &str,
        attempt_id: &str,
        reconciliation_hash: &str,
    ) -> bool {
        self.canonical_resource_bindings.contains(&(
            node_id.to_owned(),
            declaration_id.to_owned(),
            reservation_id.to_owned(),
            attempt_id.to_owned(),
            reconciliation_hash.to_owned(),
        ))
    }
}

/// The independently granted resource contract expected for one topology node.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeResourceExpectation {
    pub declaration: ResourceDeclaration,
    pub reservation: ResourceReservation,
}

#[derive(Default)]
pub struct GenericJsonlReconciler {
    reports: BTreeMap<String, (String, RuntimeNodeReport)>,
    artifacts: BTreeMap<String, (String, String, Vec<u8>)>,
    resource_allocations: BTreeMap<String, (String, RuntimeResourceAllocation)>,
    ingest_findings: Vec<IngestFinding>,
}

impl GenericJsonlReconciler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingest newline-delimited envelopes. Empty lines are ignored. Replaying
    /// identical records is idempotent; an identifier collision fails closed.
    pub fn ingest_jsonl(&mut self, input: &str) -> Vec<IngestFinding> {
        let before = self.ingest_findings.len();
        for (offset, line) in input.lines().enumerate() {
            let line_number = offset + 1;
            if line.trim().is_empty() {
                continue;
            }
            let record = match serde_json::from_str::<JsonlRecord>(line) {
                Ok(record) => record,
                Err(error) => {
                    self.push("invalid_jsonl_record", line_number, error.to_string());
                    continue;
                }
            };
            match record {
                JsonlRecord::NodeReport { report } => {
                    let source = match serde_json::to_string(&report) {
                        Ok(source) => source,
                        Err(error) => {
                            self.push("invalid_node_report", line_number, error.to_string());
                            continue;
                        }
                    };
                    let parsed = match parse_runtime_node_report(&source) {
                        Ok(report) => report,
                        Err(findings) => {
                            for finding in findings {
                                self.push(
                                    "invalid_node_report",
                                    line_number,
                                    format!("{}: {}", finding.code, finding.detail),
                                );
                            }
                            continue;
                        }
                    };
                    let canonical = canonical_runtime_node_report(&parsed)
                        .expect("typed runtime report serializes");
                    let digest = sha256(canonical.as_bytes());
                    match self.reports.get(&parsed.report_id) {
                        Some((existing, _)) if existing != &digest => self.push(
                            "report_id_collision",
                            line_number,
                            format!("{} names different report content", parsed.report_id),
                        ),
                        Some(_) => {}
                        None => {
                            self.reports
                                .insert(parsed.report_id.clone(), (digest, parsed));
                        }
                    }
                }
                JsonlRecord::Artifact {
                    artifact_id,
                    media_type,
                    content,
                } => {
                    let bytes = content.into_bytes();
                    let digest = sha256(&bytes);
                    let expected_id = format!("artifact:sha256-{digest}");
                    if artifact_id != expected_id {
                        self.push(
                            "artifact_hash_mismatch",
                            line_number,
                            format!("artifact_id must be {expected_id}"),
                        );
                        continue;
                    }
                    if media_type.trim().is_empty() {
                        self.push(
                            "empty_media_type",
                            line_number,
                            "media_type must not be empty",
                        );
                        continue;
                    }
                    match self.artifacts.get(&artifact_id) {
                        Some((existing, _, _)) if existing != &digest => self.push(
                            "artifact_id_collision",
                            line_number,
                            format!("{artifact_id} names different bytes"),
                        ),
                        Some(_) => {}
                        None => {
                            self.artifacts
                                .insert(artifact_id, (digest, media_type, bytes));
                        }
                    }
                }
                JsonlRecord::ResourceAllocation { allocation } => {
                    let parsed =
                        match serde_json::from_value::<RuntimeResourceAllocation>(allocation) {
                            Ok(allocation) => allocation,
                            Err(error) => {
                                self.push(
                                    "invalid_resource_allocation",
                                    line_number,
                                    error.to_string(),
                                );
                                continue;
                            }
                        };
                    let canonical = serde_json::to_vec(&parsed)
                        .expect("typed resource allocation serializes deterministically");
                    let digest = sha256(&canonical);
                    match self.resource_allocations.get(&parsed.allocation_id) {
                        Some((existing, _)) if existing != &digest => self.push(
                            "resource_allocation_id_collision",
                            line_number,
                            format!(
                                "{} names different allocation content",
                                parsed.allocation_id
                            ),
                        ),
                        Some(_) => {}
                        None => {
                            self.resource_allocations
                                .insert(parsed.allocation_id.clone(), (digest, parsed));
                        }
                    }
                }
            }
        }
        self.ingest_findings[before..].to_vec()
    }

    pub fn reconcile(
        &self,
        topology: &ExecutionTopology,
        base_revision_id: impl Into<String>,
    ) -> RuntimeIntegrationReport {
        self.reconcile_with_resources(topology, base_revision_id, &[])
    }

    /// Reconciles runtime observations with both graph completeness and the
    /// independently granted resource contracts. Resource decisions delegate
    /// to `resource_protocol`; this adapter only performs cross-contract joins.
    pub fn reconcile_with_resources(
        &self,
        topology: &ExecutionTopology,
        base_revision_id: impl Into<String>,
        resource_expectations: &[RuntimeResourceExpectation],
    ) -> RuntimeIntegrationReport {
        let topology_hash =
            execution_topology_content_hash(topology).expect("typed execution topology serializes");
        let lint = lint_execution_topology(topology);
        let mut local_findings = self.ingest_findings.clone();
        for finding in lint.findings.iter().filter(|finding| {
            finding.classification == FindingClassification::Deterministic
                && finding.severity == LintSeverity::Error
        }) {
            local_findings.push(IngestFinding {
                code: "topology_lint_error".to_owned(),
                line: None,
                detail: format!("{}: {}", finding.code, finding.detail),
            });
        }
        let mut nodes = Vec::new();
        for node in &topology.nodes {
            if node.outputs.len() != 1 {
                local_findings.push(IngestFinding {
                    code: "unsupported_output_cardinality".to_owned(),
                    line: None,
                    detail: format!(
                        "{} must declare exactly one v0 runtime output",
                        node.node_id
                    ),
                });
                continue;
            }
            nodes.push(ExpectedRuntimeNode {
                node_id: node.node_id.clone(),
                expected_output_schema_id: node.outputs[0].schema_id.clone(),
            });
        }
        let expectation = RuntimeGraphExpectation {
            runtime_graph_id: topology.topology_id.clone(),
            runtime_graph_content_hash: topology_hash.clone(),
            nodes,
        };
        let reports = self
            .reports
            .values()
            .map(|(_, report)| report.clone())
            .collect::<Vec<_>>();
        let observed_artifacts = self.artifacts.keys().cloned().collect::<Vec<_>>();
        for report in &reports {
            for artifact_id in &report.output_artifact_ids {
                if !is_content_addressed_artifact_id(artifact_id) {
                    local_findings.push(IngestFinding {
                        code: "invalid_output_artifact_id".to_owned(),
                        line: None,
                        detail: format!(
                            "{} declares non-content-addressed output {artifact_id}",
                            report.report_id
                        ),
                    });
                } else if !self.artifacts.contains_key(artifact_id) {
                    local_findings.push(IngestFinding {
                        code: "missing_declared_artifact".to_owned(),
                        line: None,
                        detail: format!(
                            "{} declares output {artifact_id} but no matching artifact was ingested",
                            report.report_id
                        ),
                    });
                }
            }
        }
        let completeness = reconcile_runtime_reports(&expectation, &reports, &observed_artifacts);
        let resource_reconciliations = self.reconcile_resources(
            topology,
            &reports,
            resource_expectations,
            &mut local_findings,
        );
        let resources_complete = resource_reconciliations
            .iter()
            .all(|reconciliation| reconciliation.complete);
        let resource_findings_present = local_findings.iter().any(|finding| {
            finding.code.contains("resource") || finding.code == "missing_resource_expectation"
        });
        let resource_boundary_complete = resources_complete && !resource_findings_present;
        let structurally_complete =
            completeness.complete && resource_boundary_complete && local_findings.is_empty();
        let artifacts = self
            .artifacts
            .iter()
            .map(
                |(id, (digest, media_type, bytes))| ContentAddressedArtifact {
                    artifact_id: id.clone(),
                    sha256: digest.clone(),
                    media_type: media_type.clone(),
                    byte_length: bytes.len(),
                },
            )
            .collect::<Vec<_>>();
        let proposals = if structurally_complete {
            proposals(&topology_hash, &reports, &artifacts)
        } else {
            Vec::new()
        };
        let reconciliation_hashes = resource_reconciliations
            .iter()
            .map(|reconciliation| {
                let canonical = serde_json::to_vec(reconciliation)
                    .expect("typed resource reconciliation serializes deterministically");
                (
                    (
                        reconciliation.declaration_id.clone(),
                        reconciliation.reservation_id.clone(),
                        reconciliation.attempt_id.clone(),
                    ),
                    sha256(&canonical),
                )
            })
            .collect::<BTreeMap<_, _>>();
        RuntimeIntegrationReport {
            schema: RUNTIME_INTEGRATION_REPORT_SCHEMA,
            report_version: 0,
            topology_id: topology.topology_id.clone(),
            topology_content_hash: topology_hash,
            base_revision_id: base_revision_id.into(),
            accepted: false,
            reconciliation_complete: structurally_complete,
            halt: if structurally_complete {
                IntegrationHalt::NeedsReview
            } else if completeness.complete && !resource_boundary_complete {
                IntegrationHalt::ResourceReconciliationIncomplete
            } else {
                IntegrationHalt::IncompleteRuntimeReports
            },
            completeness,
            resource_reconciliations,
            artifacts,
            proposals,
            ingest_findings: local_findings,
            canonical_resource_bindings: resource_expectations
                .iter()
                .map(|expectation| {
                    (
                        expectation.declaration.node_id.clone(),
                        expectation.declaration.declaration_id.clone(),
                        expectation.reservation.reservation_id.clone(),
                        expectation.reservation.attempt_id.clone(),
                        reconciliation_hashes
                            .get(&(
                                expectation.declaration.declaration_id.clone(),
                                expectation.reservation.reservation_id.clone(),
                                expectation.reservation.attempt_id.clone(),
                            ))
                            .cloned()
                            .unwrap_or_default(),
                    )
                })
                .collect(),
        }
    }

    fn reconcile_resources(
        &self,
        topology: &ExecutionTopology,
        reports: &[RuntimeNodeReport],
        expectations: &[RuntimeResourceExpectation],
        findings: &mut Vec<IngestFinding>,
    ) -> Vec<ResourceReconciliation> {
        let mut by_node = BTreeMap::new();
        for expectation in expectations {
            if by_node
                .insert(expectation.declaration.node_id.as_str(), expectation)
                .is_some()
            {
                findings.push(integration_finding(
                    "duplicate_resource_expectation",
                    format!(
                        "{} has more than one resource expectation",
                        expectation.declaration.node_id
                    ),
                ));
            }
        }
        for node in topology
            .nodes
            .iter()
            .filter(|node| !node.resource_claims.is_empty())
        {
            if !by_node.contains_key(node.node_id.as_str()) {
                findings.push(integration_finding(
                    "missing_resource_expectation",
                    format!(
                        "{} declares resources but has no declaration/reservation expectation",
                        node.node_id
                    ),
                ));
            }
        }

        let allocations = self
            .resource_allocations
            .values()
            .map(|(_, allocation)| allocation)
            .collect::<Vec<_>>();
        let mut consumed = BTreeSet::new();
        let mut reconciliations = Vec::new();
        for expectation in expectations {
            for finding in validate_resource_declaration(topology, &expectation.declaration) {
                findings.push(integration_finding(
                    "resource_declaration_mismatch",
                    format!("{}: {}", finding.code, finding.detail),
                ));
            }
            if !reports.iter().any(|report| {
                report.node_id == expectation.declaration.node_id
                    && report.attempt_id == expectation.reservation.attempt_id
            }) {
                findings.push(integration_finding(
                    "resource_attempt_report_mismatch",
                    format!(
                        "{}/{} does not join a runtime node report",
                        expectation.declaration.node_id, expectation.reservation.attempt_id
                    ),
                ));
            }
            let matching = allocations
                .iter()
                .filter(|allocation| {
                    allocation.reservation_id == expectation.reservation.reservation_id
                        || allocation.attempt_id == expectation.reservation.attempt_id
                })
                .map(|allocation| {
                    consumed.insert(allocation.allocation_id.as_str());
                    (*allocation).clone()
                })
                .collect::<Vec<_>>();
            let reconciliation = reconcile_resource_allocations(
                &expectation.declaration,
                &expectation.reservation,
                &matching,
            );
            for finding in &reconciliation.findings {
                findings.push(integration_finding(
                    "resource_reconciliation_mismatch",
                    format!("{}: {}", finding.code, finding.detail),
                ));
            }
            reconciliations.push(reconciliation);
        }
        for allocation in allocations {
            if !consumed.contains(allocation.allocation_id.as_str()) {
                findings.push(integration_finding(
                    "unaccounted_resource_allocation",
                    format!(
                        "{} does not join any expected declaration/reservation",
                        allocation.allocation_id
                    ),
                ));
            }
        }
        reconciliations.sort_by(|left, right| {
            (&left.declaration_id, &left.reservation_id, &left.attempt_id).cmp(&(
                &right.declaration_id,
                &right.reservation_id,
                &right.attempt_id,
            ))
        });
        reconciliations
    }

    fn push(&mut self, code: &str, line: usize, detail: impl Into<String>) {
        self.ingest_findings.push(IngestFinding {
            code: code.to_owned(),
            line: Some(line),
            detail: detail.into(),
        });
    }
}

fn integration_finding(code: &str, detail: impl Into<String>) -> IngestFinding {
    IngestFinding {
        code: code.to_owned(),
        line: None,
        detail: detail.into(),
    }
}

fn proposals(
    graph_hash: &str,
    reports: &[RuntimeNodeReport],
    artifacts: &[ContentAddressedArtifact],
) -> Vec<IntegrationProposal> {
    let successful_outputs = reports
        .iter()
        .filter(|report| report.status == RuntimeNodeStatus::Succeeded)
        .flat_map(|report| report.output_artifact_ids.iter())
        .collect::<BTreeSet<_>>();
    let mut result = artifacts
        .iter()
        .filter(|artifact| successful_outputs.contains(&artifact.artifact_id))
        .map(|artifact| {
            proposal(
                ProposalKind::Evidence,
                json!({
                    "artifact_id": artifact.artifact_id,
                    "content_hash": artifact.sha256,
                    "evidence_boundary": "worker_output",
                    "runtime_claim_accepted": false
                }),
            )
        })
        .collect::<Vec<_>>();
    result.push(proposal(
        ProposalKind::Morphism,
        json!({
            "morphism_type": "runtime_reconciliation",
            "runtime_graph_content_hash": graph_hash,
            "runtime_claim_accepted": false
        }),
    ));
    result.sort_by(|left, right| left.proposal_id.cmp(&right.proposal_id));
    result
}

fn proposal(kind: ProposalKind, payload: Value) -> IntegrationProposal {
    let canonical = serde_json::to_vec(&(kind, &payload)).expect("proposal serializes");
    IntegrationProposal {
        proposal_id: format!("proposal:sha256-{}", sha256(&canonical)),
        kind,
        review_status: ProposalReviewStatus::Unreviewed,
        source_boundary: "external_runtime_untrusted".to_owned(),
        payload,
    }
}

fn sha256(bytes: &[u8]) -> String {
    crate::native_hash::sha256_hex(bytes)
}

fn is_content_addressed_artifact_id(value: &str) -> bool {
    value.strip_prefix("artifact:sha256-").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

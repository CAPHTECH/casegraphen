//! Crosses the JSON-Schema/Rust boundary for experimental v0 contracts.
//!
//! The Python gate owns inventory/reference resolution. This test proves that
//! representative Rust-owned inputs and records deserialize and serialize back
//! into instances accepted by the same shipped schemas.

use casegraphen::{
    control_plane::{ControlPlaneNotification, ControlPlaneRequest, ControlPlaneResponse},
    dynamic_expansion::{canonical_topology_patch, ExpansionPolicy, TopologyPatch},
    execution_topology::ExecutionTopology,
    github_evidence::{
        CaptureManifest, CheckEvidence, PrObservation, RefreshResult, ReviewFinding,
        ReviewIndependence, ReviewProjection,
    },
    graph_simulation::GraphSimulationRequest,
    memory::{
        MemoryClaim, MemoryIndex, MemoryPolicy, MemoryProjection, MemoryQuery, MemoryUseReport,
        SourceRecord,
    },
    native_review::ExecutionTopologyReviewArtifact,
    resource_allocator::{
        ResourceAllocatorCheckpoint, ResourceAllocatorCompactionRecord,
        ResourceAllocatorConfiguration, ResourceAllocatorEvent, ResourceAllocatorRetentionPolicy,
        ReviewedDeploymentReservationBinding,
    },
    resource_protocol::{
        GitWorktreeRecord, RateLimitCapacity, ReservationDispositionAssertion, ResourceDeclaration,
        ResourceReconciliation, ResourceReservation, RuntimeResourceAllocation,
    },
    runtime_integration::ResourceExpectationBundle,
    runtime_protocol::RuntimeNodeReport,
    streaming_reconciliation::RuntimeStreamEvent,
    verification_policy::{VerificationLineageDeclarations, VerificationPolicy},
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

fn roundtrip<T: DeserializeOwned + Serialize>(relative: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("schemas/experimental")
        .join(relative);
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let typed: T = serde_json::from_str(&source)
        .unwrap_or_else(|error| panic!("deserialize {} into Rust owner: {error}", path.display()));
    let serialized = serde_json::to_value(&typed).expect("Rust owner serializes");
    let reparsed: T = serde_json::from_value(serialized.clone()).unwrap_or_else(|error| {
        panic!("reparse Rust serialization for {}: {error}", path.display())
    });
    serde_json::to_value(reparsed).expect("reparsed Rust owner serializes")
}

#[test]
fn inventory_examples_references_and_negative_fixtures_are_gated() {
    let status = Command::new("python3")
        .arg("scripts/experimental-schema-conformance.py")
        .args(["--check", "--self-test"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("run experimental schema conformance gate");
    assert!(
        status.success(),
        "experimental schema conformance gate failed"
    );
}

#[test]
fn representative_rust_roundtrips_validate_against_shipped_schemas() {
    let instances = vec![
        json!({"schema_id":"casegraphen.experimental.execution.topology.v0","instance":roundtrip::<ExecutionTopology>("execution.topology.file-review.example.json")}),
        json!({"schema_id":"casegraphen.experimental.execution_topology_review.v0","instance":roundtrip::<ExecutionTopologyReviewArtifact>("execution_topology.review.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.expansion.policy.v0","instance":roundtrip::<ExpansionPolicy>("expansion.policy.example.json")}),
        json!({"schema_id":"casegraphen.experimental.topology.patch.v0","instance":roundtrip::<TopologyPatch>("topology.patch.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.graph_simulation.request.v0","instance":roundtrip::<GraphSimulationRequest>("graph_simulation.request.example.json")}),
        json!({"schema_id":"casegraphen.experimental.memory.source_record.v0","instance":roundtrip::<SourceRecord>("memory.source_record.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.memory.claim.v0","instance":roundtrip::<MemoryClaim>("memory.claim.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.memory.query.v0","instance":roundtrip::<MemoryQuery>("memory.query.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.memory.policy.v0","instance":roundtrip::<MemoryPolicy>("memory.policy.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.memory.projection.v0","instance":roundtrip::<MemoryProjection>("memory.projection.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.memory.use_report.v0","instance":roundtrip::<MemoryUseReport>("memory.use_report.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.memory.index.v0","instance":roundtrip::<MemoryIndex>("memory.index.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.declaration.v0","instance":roundtrip::<ResourceDeclaration>("resource.declaration.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.reservation.v0","instance":roundtrip::<ResourceReservation>("resource.reservation.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.reservation_disposition.v0","instance":roundtrip::<ReservationDispositionAssertion>("resource.reservation_disposition.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.rate_limit_capacity.v0","instance":roundtrip::<RateLimitCapacity>("resource.rate_limit_capacity.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.allocator_configuration.v0","instance":roundtrip::<ResourceAllocatorConfiguration>("resource.allocator_configuration.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.allocator_event.v0","instance":roundtrip::<ResourceAllocatorEvent>("resource.allocator_event.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.allocator_checkpoint.v0","instance":roundtrip::<ResourceAllocatorCheckpoint>("resource.allocator_checkpoint.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.allocator_retention_policy.v0","instance":roundtrip::<ResourceAllocatorRetentionPolicy>("resource.allocator_retention_policy.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.allocator_compaction.v0","instance":roundtrip::<ResourceAllocatorCompactionRecord>("resource.allocator_compaction.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.reviewed_deployment_binding.v0","instance":roundtrip::<ReviewedDeploymentReservationBinding>("resource.reviewed_deployment_binding.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.runtime.resource_allocation.v0","instance":roundtrip::<RuntimeResourceAllocation>("runtime.resource_allocation.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.resource.reconciliation.v0","instance":roundtrip::<ResourceReconciliation>("resource.reconciliation.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.git.worktree_record.v0","instance":roundtrip::<GitWorktreeRecord>("git.worktree_record.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.runtime.node_report.v0","instance":roundtrip::<RuntimeNodeReport>("runtime.node_report.example.json")}),
        json!({"schema_id":"casegraphen.experimental.runtime.resource_expectation_bundle.v0","instance":roundtrip::<ResourceExpectationBundle>("runtime.resource_expectation_bundle.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.runtime.stream_event.v0","instance":roundtrip::<RuntimeStreamEvent>("runtime.stream_event.example.json")}),
        json!({"schema_id":"casegraphen.experimental.verification_policy.v0","instance":roundtrip::<VerificationPolicy>("verification.policy.example.json")}),
        json!({"schema_id":"casegraphen.experimental.verification_lineage_declarations.v0","instance":roundtrip::<VerificationLineageDeclarations>("verification_lineage_declarations.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.control_plane.request.v0","instance":roundtrip::<ControlPlaneRequest>("control_plane.request.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.control_plane.response.v0","instance":roundtrip::<ControlPlaneResponse>("control_plane.response.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.control_plane.notification.v0","instance":roundtrip::<ControlPlaneNotification>("control_plane.notification.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.github.capture_manifest.v0","instance":roundtrip::<CaptureManifest>("github.capture_manifest.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.github.pr_observation.v0","instance":roundtrip::<PrObservation>("github.pr_observation.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.github.check_evidence.v0","instance":roundtrip::<CheckEvidence>("github.check_evidence.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.github.review_finding.v0","instance":roundtrip::<ReviewFinding>("github.review_finding.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.github.review_independence.v0","instance":roundtrip::<ReviewIndependence>("github.review_independence.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.github.refresh_result.v0","instance":roundtrip::<RefreshResult>("github.refresh_result.v0.example.json")}),
        json!({"schema_id":"casegraphen.experimental.github.review_projection.v0","instance":roundtrip::<ReviewProjection>("github.review_projection.v0.example.json")}),
    ];
    let bundle = std::env::temp_dir().join(format!(
        "casegraphen-experimental-schema-instances-{}.json",
        std::process::id()
    ));
    fs::write(&bundle, serde_json::to_vec(&instances).unwrap()).expect("write instance bundle");
    let status = Command::new("python3")
        .arg("scripts/experimental-schema-conformance.py")
        .arg("--instances")
        .arg(&bundle)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("validate Rust serialization against JSON Schema");
    let _ = fs::remove_file(&bundle);
    assert!(
        status.success(),
        "Rust serialization did not match shipped JSON Schema"
    );
}

#[test]
fn rust_boundary_deserializers_reject_unknown_fields_and_stale_versions() {
    let mut patch: Value = serde_json::from_str(include_str!(
        "../schemas/experimental/topology.patch.v0.example.json"
    ))
    .unwrap();
    patch["schema_version"] = json!(1);
    let typed: TopologyPatch = serde_json::from_value(patch).expect("wire shape remains parseable");
    let base: ExecutionTopology = serde_json::from_str(include_str!(
        "../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .unwrap();
    let findings =
        canonical_topology_patch(&base, &typed).expect_err("stale patch version refuses");
    assert!(findings
        .iter()
        .any(|finding| finding.code == "unsupported_topology_patch_schema"));

    let mut policy: Value = serde_json::from_str(include_str!(
        "../schemas/experimental/expansion.policy.example.json"
    ))
    .unwrap();
    policy["unknown"] = json!(true);
    assert!(serde_json::from_value::<ExpansionPolicy>(policy).is_err());
}

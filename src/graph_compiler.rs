//! Deterministic lowering from a lint-clean execution-topology proposal.
//!
//! Compilation is deliberately pure: it emits content-addressed bytes but does
//! not write a store, accept a review, reserve a resource, or dispatch work.

use crate::{
    deployment_policy::{
        deployment_policy_manifest, deployment_policy_manifest_content_hash,
        validate_deployment_policy_manifest,
    },
    dynamic_expansion::{validate_expansion_policy, ExpansionPolicy},
    exec::{validate_execution_plan, AllowedTransitionClass, ExecutionPlan},
    execution_topology::{
        canonical_execution_topology, execution_topology_content_hash, ExecutionTopology,
    },
    graph_lint::{lint_execution_topology_with_verification_policies, GraphLintReport},
    native_eval::validate_native_case_space,
    native_model::{CaseCellType, CaseSpace, ReviewAction},
    native_review::{canonical_review, NativeReviewTargetKind},
    verification_policy::parse_verification_policy,
};
use higher_graphen_core::ReviewStatus;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Version of the deterministic compiler implementation.
pub const GRAPH_COMPILER_VERSION: &str = "casegraphen-graph-compiler/0";
/// Schema identity of the deployment-bundle manifest.
pub const DEPLOYMENT_BUNDLE_SCHEMA: &str = "casegraphen.experimental.deployment_bundle.v0";
/// Schema identity of compiler reports.
pub const COMPILER_REPORT_SCHEMA: &str = "casegraphen.experimental.graph_compiler.report.v0";
/// Schema identity of the retained canonical inputs used to prove compiler provenance.
pub const COMPILER_INPUTS_SCHEMA: &str = "casegraphen.experimental.graph_compiler.inputs.v0";

/// Compilation for inspection, or compilation tied to a canonical accepted review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompilationMode {
    Proposal,
    Reviewed(ReviewedTopologyBinding),
}

/// Opaque proof that a topology hash was bound to a reviewed case claim.
///
/// Callers cannot construct this value directly. [`reviewed_compilation_mode`]
/// derives it from the canonical CaseGraphen review log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedTopologyBinding {
    claim_cell_id: String,
    review_id: String,
    topology_content_hash: String,
    policy_manifest_content_hash: String,
    case_space_id: String,
    base_revision_id: String,
    expansion_proposal_id: Option<String>,
}

impl ReviewedTopologyBinding {
    pub(crate) fn review_id(&self) -> &str {
        &self.review_id
    }

    pub(crate) fn topology_content_hash(&self) -> &str {
        &self.topology_content_hash
    }

    pub(crate) fn base_revision_id(&self) -> &str {
        &self.base_revision_id
    }

    pub(crate) fn expansion_proposal_id(&self) -> Option<&str> {
        self.expansion_proposal_id.as_deref()
    }
}

impl CompilationMode {
    pub(crate) fn reviewed_binding(&self) -> Option<&ReviewedTopologyBinding> {
        match self {
            Self::Proposal => None,
            Self::Reviewed(binding) => Some(binding),
        }
    }
}

/// The supported deployment lowering. Generic JSONL transports reports; it
/// does not gain authority over CaseGraphen acceptance.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilationTarget {
    GenericJsonlV0,
}

/// Explicit mapping needed to create a real, still-unreviewed execution plan.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodePlanMapping {
    pub node_id: String,
    pub worker_binding_id: String,
    pub success_evidence_requirement_ids: Vec<String>,
    pub allowed_transition_classes: Vec<AllowedTransitionClass>,
}

/// Inputs that are deployment-specific rather than topology semantics.
#[derive(Clone, Debug, PartialEq)]
pub struct CompilerRequest {
    pub mode: CompilationMode,
    pub target: CompilationTarget,
    pub case_space_id: String,
    pub base_revision_id: String,
    pub plan_id: String,
    pub node_plan_mappings: Vec<NodePlanMapping>,
    pub verification_policies: BTreeMap<String, Value>,
    pub budget_policies: BTreeMap<String, Value>,
    pub expansion_policies: BTreeMap<String, Value>,
}

/// Serializable authority-free representation of the compiler mode. Reviewed
/// fields are retained so verification can reproduce the exact compiler run;
/// deserializing this value does not itself mint an opaque review proof.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RetainedCompilationMode {
    Proposal,
    Reviewed {
        claim_cell_id: String,
        review_id: String,
        topology_content_hash: String,
        policy_manifest_content_hash: String,
        case_space_id: String,
        base_revision_id: String,
        expansion_proposal_id: Option<String>,
    },
}

/// Canonical compiler inputs retained inside every bundle. This record is
/// untrusted until the verifier deterministically recompiles it and compares
/// every generated artifact byte.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CompilerInputsArtifact {
    schema: String,
    compiler_version: String,
    mode: RetainedCompilationMode,
    target: CompilationTarget,
    case_space_id: String,
    base_revision_id: String,
    plan_id: String,
    node_plan_mappings: Vec<NodePlanMapping>,
    verification_policies: BTreeMap<String, Value>,
    budget_policies: BTreeMap<String, Value>,
    expansion_policies: BTreeMap<String, Value>,
}

/// Severity of semantics that could not be preserved during lowering.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InformationLossSeverity {
    Representational,
    SafetyAffecting,
    AcceptanceAffecting,
}

/// An explicit source contract that a target could not represent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompilerInformationLoss {
    pub code: String,
    pub severity: InformationLossSeverity,
    pub source_ids: Vec<String>,
    pub detail: String,
}

/// Stable compiler diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompilerFinding {
    pub code: String,
    pub location: String,
    pub detail: String,
}

/// Outcome written into a compiler report.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilerStatus {
    Compiled,
    Refused,
}

/// Deterministic report returned on both success and refusal.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CompilerReport {
    pub schema: &'static str,
    pub compiler_version: &'static str,
    pub status: CompilerStatus,
    pub mode: String,
    pub target: CompilationTarget,
    pub topology_id: String,
    pub topology_content_hash: String,
    pub case_space_id: String,
    pub base_revision_id: String,
    pub reviewed_claim_cell_id: Option<String>,
    pub accepted_review_id: Option<String>,
    pub generated_plan_review_status: &'static str,
    pub unsupported_semantics: Vec<CompilerFinding>,
    pub information_loss: Vec<CompilerInformationLoss>,
}

/// One content-addressed file in a deployment bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleArtifact {
    pub path: String,
    pub content_hash: String,
    pub bytes: Vec<u8>,
}

/// Manifest entry joining a path to exact bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifestEntry {
    pub path: String,
    pub content_hash: String,
    pub byte_length: u64,
}

/// Manifest joining every generated artifact to topology and revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub schema: String,
    pub compiler_version: String,
    pub topology_id: String,
    pub topology_content_hash: String,
    pub case_space_id: String,
    pub base_revision_id: String,
    pub mode: String,
    pub policy_manifest_content_hash: String,
    pub reviewed_claim_cell_id: Option<String>,
    pub accepted_review_id: Option<String>,
    pub accepted_review_revision_id: Option<String>,
    pub artifacts: Vec<BundleManifestEntry>,
}

/// A byte-stable bundle. The manifest itself is content-addressed separately
/// to avoid a recursive self-hash field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentBundle {
    pub artifacts: Vec<BundleArtifact>,
    pub manifest: BundleManifest,
    pub manifest_bytes: Vec<u8>,
    pub manifest_content_hash: String,
}

/// Opaque proof that a bundle is byte-integral and exactly reproducible by the
/// canonical deterministic compiler from its retained inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedDeploymentBundle {
    bundle: DeploymentBundle,
    topology: ExecutionTopology,
}

impl VerifiedDeploymentBundle {
    pub fn manifest(&self) -> &BundleManifest {
        &self.bundle.manifest
    }

    pub fn manifest_content_hash(&self) -> &str {
        &self.bundle.manifest_content_hash
    }

    pub fn artifact_bytes(&self, path: &str) -> Option<&[u8]> {
        self.bundle
            .artifacts
            .iter()
            .find(|artifact| artifact.path == path)
            .map(|artifact| artifact.bytes.as_slice())
    }

    pub fn topology(&self) -> &ExecutionTopology {
        &self.topology
    }
}

const REQUIRED_DEPLOYMENT_ARTIFACTS: [&str; 12] = [
    "execution.topology.json",
    "topology.content-hash",
    "compiler.inputs.json",
    "compiler.report.json",
    "case.mapping.genesis.proposal.json",
    "execution.plan.proposal.json",
    "runtime.deployment.json",
    "resource.manifest.json",
    "verification.policies.json",
    "budget.policies.json",
    "expansion.policies.json",
    "graph.analysis.report.json",
];

/// Verify a persisted compiler bundle before it can participate in authority
/// derivation. Callers cannot turn a syntactically valid digest into a proof.
pub fn verify_deployment_bundle(
    bundle: DeploymentBundle,
    expected_manifest_content_hash: &str,
) -> Result<VerifiedDeploymentBundle, CompilerFinding> {
    let invalid = |location: &str, detail: &str| {
        compiler_finding("deployment_bundle_integrity_failure", location, detail)
    };
    if expected_manifest_content_hash.len() != 64
        || !expected_manifest_content_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || bundle.manifest_content_hash != expected_manifest_content_hash
        || crate::native_hash::sha256_hex(&bundle.manifest_bytes) != expected_manifest_content_hash
    {
        return Err(invalid(
            "$.manifest",
            "manifest bytes do not match the requested content address",
        ));
    }
    let parsed: BundleManifest = serde_json::from_slice(&bundle.manifest_bytes)
        .map_err(|error| invalid("$.manifest", &error.to_string()))?;
    if parsed != bundle.manifest {
        return Err(invalid(
            "$.manifest",
            "parsed manifest differs from the supplied typed manifest",
        ));
    }
    let manifest_entries = bundle
        .manifest
        .artifacts
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let artifacts = bundle
        .artifacts
        .iter()
        .map(|artifact| (artifact.path.as_str(), artifact))
        .collect::<BTreeMap<_, _>>();
    if manifest_entries.len() != bundle.manifest.artifacts.len()
        || artifacts.len() != bundle.artifacts.len()
        || manifest_entries.keys().ne(artifacts.keys())
        || REQUIRED_DEPLOYMENT_ARTIFACTS
            .iter()
            .any(|path| !artifacts.contains_key(path))
    {
        return Err(invalid(
            "$.manifest.artifacts",
            "artifact inventory is duplicated, incomplete, or differs from the manifest",
        ));
    }
    for (path, artifact) in artifacts {
        let entry = manifest_entries[path];
        let actual_hash = crate::native_hash::sha256_hex(&artifact.bytes);
        if artifact.content_hash != actual_hash
            || entry.content_hash != actual_hash
            || entry.byte_length != artifact.bytes.len() as u64
        {
            return Err(invalid(
                &format!("$.artifacts[{path}]"),
                "artifact bytes, digest, and manifest entry do not agree",
            ));
        }
    }
    let topology_bytes = bundle
        .artifacts
        .iter()
        .find(|artifact| artifact.path == "execution.topology.json")
        .expect("required inventory was checked")
        .bytes
        .as_slice();
    let topology_text = std::str::from_utf8(topology_bytes)
        .map_err(|error| invalid("$.artifacts[execution.topology.json]", &error.to_string()))?;
    let topology =
        crate::execution_topology::parse_execution_topology(topology_text).map_err(|findings| {
            invalid(
                "$.artifacts[execution.topology.json]",
                &serde_json::to_string(&findings).expect("topology findings serialize"),
            )
        })?;
    let topology_hash = execution_topology_content_hash(&topology)
        .expect("validated execution topology serializes");
    if topology.topology_id != bundle.manifest.topology_id
        || topology.case_space_id != bundle.manifest.case_space_id
        || topology_hash != bundle.manifest.topology_content_hash
    {
        return Err(invalid(
            "$.artifacts[execution.topology.json]",
            "topology artifact identity or content hash differs from the manifest",
        ));
    }

    let inputs_bytes = bundle
        .artifacts
        .iter()
        .find(|artifact| artifact.path == "compiler.inputs.json")
        .expect("required inventory was checked")
        .bytes
        .as_slice();
    let retained: CompilerInputsArtifact =
        serde_json::from_slice(inputs_bytes).map_err(|error| {
            compiler_finding(
                "deployment_bundle_semantic_mismatch",
                "$.artifacts[compiler.inputs.json]",
                error.to_string(),
            )
        })?;
    let request = retained_compiler_request(retained).map_err(|detail| {
        compiler_finding(
            "deployment_bundle_semantic_mismatch",
            "$.artifacts[compiler.inputs.json]",
            detail,
        )
    })?;
    let reproduced = compile_execution_topology(&topology, &request).map_err(|report| {
        compiler_finding(
            "deployment_bundle_semantic_mismatch",
            "$.artifacts",
            format!(
                "retained inputs do not produce a deployable bundle: {}",
                serde_json::to_string(&report).expect("compiler report serializes")
            ),
        )
    })?;
    if !deployment_bundles_match(&bundle, &reproduced) {
        return Err(compiler_finding(
            "deployment_bundle_semantic_mismatch",
            "$.artifacts",
            "bundle artifact bytes or manifest differ from deterministic compiler output",
        ));
    }
    Ok(VerifiedDeploymentBundle { bundle, topology })
}

fn retained_compiler_request(retained: CompilerInputsArtifact) -> Result<CompilerRequest, String> {
    if retained.schema != COMPILER_INPUTS_SCHEMA {
        return Err("compiler input schema is unsupported".to_owned());
    }
    if retained.compiler_version != GRAPH_COMPILER_VERSION {
        return Err("compiler input version differs from this verifier".to_owned());
    }
    let mode = match retained.mode {
        RetainedCompilationMode::Proposal => CompilationMode::Proposal,
        RetainedCompilationMode::Reviewed {
            claim_cell_id,
            review_id,
            topology_content_hash,
            policy_manifest_content_hash,
            case_space_id,
            base_revision_id,
            expansion_proposal_id,
        } => CompilationMode::Reviewed(ReviewedTopologyBinding {
            claim_cell_id,
            review_id,
            topology_content_hash,
            policy_manifest_content_hash,
            case_space_id,
            base_revision_id,
            expansion_proposal_id,
        }),
    };
    Ok(CompilerRequest {
        mode,
        target: retained.target,
        case_space_id: retained.case_space_id,
        base_revision_id: retained.base_revision_id,
        plan_id: retained.plan_id,
        node_plan_mappings: retained.node_plan_mappings,
        verification_policies: retained.verification_policies,
        budget_policies: retained.budget_policies,
        expansion_policies: retained.expansion_policies,
    })
}

fn deployment_bundles_match(left: &DeploymentBundle, right: &DeploymentBundle) -> bool {
    let artifact_map = |bundle: &DeploymentBundle| {
        bundle
            .artifacts
            .iter()
            .map(|artifact| {
                (
                    artifact.path.clone(),
                    (artifact.content_hash.clone(), artifact.bytes.clone()),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    left.manifest == right.manifest
        && left.manifest_bytes == right.manifest_bytes
        && left.manifest_content_hash == right.manifest_content_hash
        && artifact_map(left) == artifact_map(right)
}

/// Opaque proof that a persisted deployment bundle is exactly the output
/// authorized by the latest canonical execution-topology review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedDeploymentAuthority {
    claim_cell_id: String,
    accepted_review_id: String,
    topology_content_hash: String,
    policy_manifest_content_hash: String,
    deployment_bundle_hash: String,
    accepted_review_revision_id: String,
    case_space_id: String,
}

impl ReviewedDeploymentAuthority {
    pub(crate) fn claim_cell_id(&self) -> &str {
        &self.claim_cell_id
    }

    pub(crate) fn accepted_review_id(&self) -> &str {
        &self.accepted_review_id
    }

    pub(crate) fn topology_content_hash(&self) -> &str {
        &self.topology_content_hash
    }

    pub(crate) fn policy_manifest_content_hash(&self) -> &str {
        &self.policy_manifest_content_hash
    }

    pub(crate) fn deployment_bundle_hash(&self) -> &str {
        &self.deployment_bundle_hash
    }

    pub(crate) fn accepted_review_revision_id(&self) -> &str {
        &self.accepted_review_revision_id
    }

    pub(crate) fn case_space_id(&self) -> &str {
        &self.case_space_id
    }
}

/// Re-derives deployment authority from canonical review state and a verified
/// content-addressed bundle manifest. The manifest bytes and artifact entries
/// must be hash-verified by the persistence adapter before this call.
pub fn reviewed_deployment_authority(
    case_space: &CaseSpace,
    claim_cell_id: &str,
    bundle: &VerifiedDeploymentBundle,
) -> Result<ReviewedDeploymentAuthority, CompilerFinding> {
    let manifest = bundle.manifest();
    let deployment_bundle_hash = bundle.manifest_content_hash();
    let CompilationMode::Reviewed(binding) = reviewed_compilation_mode(case_space, claim_cell_id)?
    else {
        unreachable!("reviewed_compilation_mode only returns reviewed authority")
    };
    let mismatch = |location: &str, detail: &str| {
        compiler_finding("reviewed_deployment_binding_mismatch", location, detail)
    };
    if manifest.schema != DEPLOYMENT_BUNDLE_SCHEMA
        || manifest.compiler_version != GRAPH_COMPILER_VERSION
        || manifest.mode != "reviewed"
    {
        return Err(mismatch(
            "$.manifest",
            "bundle is not a reviewed deployment manifest from this compiler version",
        ));
    }
    if manifest.case_space_id != binding.case_space_id
        || manifest.base_revision_id != binding.base_revision_id
        || manifest.accepted_review_revision_id.as_deref()
            != Some(binding.base_revision_id.as_str())
    {
        return Err(mismatch(
            "$.manifest.base_revision_id",
            "bundle is not bound to the accepted review revision",
        ));
    }
    if manifest.topology_content_hash != binding.topology_content_hash {
        return Err(mismatch(
            "$.manifest.topology_content_hash",
            "bundle topology differs from the accepted topology review",
        ));
    }
    if manifest.policy_manifest_content_hash != binding.policy_manifest_content_hash {
        return Err(mismatch(
            "$.manifest.policy_manifest_content_hash",
            "bundle policies differ from the accepted topology review",
        ));
    }
    if manifest.reviewed_claim_cell_id.as_deref() != Some(claim_cell_id)
        || manifest.accepted_review_id.as_deref() != Some(binding.review_id.as_str())
    {
        return Err(mismatch(
            "$.manifest.accepted_review_id",
            "bundle does not name the canonical accepted topology review",
        ));
    }
    if deployment_bundle_hash.len() != 64
        || !deployment_bundle_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(compiler_finding(
            "invalid_deployment_bundle_hash",
            "$.deployment_bundle_hash",
            "deployment bundle hash must be a lowercase SHA-256 hex digest",
        ));
    }
    Ok(ReviewedDeploymentAuthority {
        claim_cell_id: claim_cell_id.to_owned(),
        accepted_review_id: binding.review_id,
        topology_content_hash: binding.topology_content_hash,
        policy_manifest_content_hash: binding.policy_manifest_content_hash,
        deployment_bundle_hash: deployment_bundle_hash.to_owned(),
        accepted_review_revision_id: binding.base_revision_id,
        case_space_id: binding.case_space_id,
    })
}

/// Derive reviewed mode from the canonical, validated CaseGraphen review log.
pub fn reviewed_compilation_mode(
    case_space: &CaseSpace,
    claim_cell_id: &str,
) -> Result<CompilationMode, CompilerFinding> {
    validate_native_case_space(case_space).map_err(|error| {
        compiler_finding(
            "invalid_case_space",
            "$.mode",
            format!("reviewed compilation requires a valid case space: {error:?}"),
        )
    })?;
    let claim = case_space
        .case_cells
        .iter()
        .find(|cell| cell.id.as_str() == claim_cell_id)
        .ok_or_else(|| {
            compiler_finding(
                "unknown_topology_claim",
                "$.mode.claim_cell_id",
                "claim_cell_id does not exist in the case space",
            )
        })?;
    let is_topology_claim = match &claim.cell_type {
        CaseCellType::Evidence => true,
        CaseCellType::Custom(kind) => kind == "execution_topology",
        _ => false,
    };
    if !is_topology_claim {
        return Err(compiler_finding(
            "invalid_topology_claim_type",
            "$.mode.claim_cell_id",
            "reviewed topology binding must target evidence or execution_topology claim",
        ));
    }
    let latest = case_space
        .morphism_log
        .iter()
        .filter_map(|entry| canonical_review(&entry.morphism).map(|review| (entry, review)))
        .rfind(|(_, review)| {
            review.target_kind == NativeReviewTargetKind::ExecutionTopology
                && review.target_id.as_str() == claim_cell_id
        })
        .ok_or_else(|| {
            compiler_finding(
                "topology_claim_unreviewed",
                "$.mode.claim_cell_id",
                "claim has no canonical review",
            )
        })?;
    if latest.1.target_kind != NativeReviewTargetKind::ExecutionTopology
        || latest.1.action != ReviewAction::Accept
        || latest.1.outcome != ReviewStatus::Accepted
    {
        return Err(compiler_finding(
            "topology_claim_not_accepted",
            "$.mode.claim_cell_id",
            "latest canonical review is not an execution-topology acceptance",
        ));
    }
    let target = latest.1.execution_topology.as_ref().ok_or_else(|| {
        compiler_finding(
            "malformed_topology_review_binding",
            "$.mode.claim_cell_id",
            "accepted execution-topology review does not retain its exact binding",
        )
    })?;
    if target.claim_cell_id.as_str() != claim_cell_id {
        return Err(compiler_finding(
            "topology_review_claim_mismatch",
            "$.mode.claim_cell_id",
            "review target and requested claim differ",
        ));
    }
    let review_id = latest
        .0
        .morphism
        .metadata
        .get("review_id")
        .and_then(Value::as_str)
        .expect("canonical_review validates review_id")
        .to_owned();

    Ok(CompilationMode::Reviewed(ReviewedTopologyBinding {
        claim_cell_id: claim_cell_id.to_owned(),
        review_id,
        topology_content_hash: target.topology_content_hash.clone(),
        policy_manifest_content_hash: target.policy_manifest_content_hash.clone(),
        case_space_id: target.case_space_id.to_string(),
        // The reviewer observed the topology at `observed_base_revision_id`,
        // retained inside the canonical target. Deployment starts from the
        // review morphism's target revision, where that acceptance actually
        // exists; using the observed predecessor would generate a plan that
        // the current ledger immediately rejects as stale.
        base_revision_id: latest.0.target_revision_id.to_string(),
        expansion_proposal_id: target
            .expansion_proposal_id
            .as_ref()
            .map(ToString::to_string),
    }))
}

/// Compile a topology without mutating or accepting any CaseGraphen state.
pub fn compile_execution_topology(
    topology: &ExecutionTopology,
    request: &CompilerRequest,
) -> Result<DeploymentBundle, Box<CompilerReport>> {
    let canonical_topology = canonical_execution_topology(topology)
        .expect("typed execution topology serializes deterministically");
    let canonical_topology: ExecutionTopology =
        serde_json::from_str(&canonical_topology).expect("canonical typed topology deserializes");
    let topology = &canonical_topology;
    let topology_hash = execution_topology_content_hash(topology)
        .expect("canonical execution topology hashes deterministically");
    let (mode_name, reviewed_claim, accepted_review, accepted_review_revision) =
        mode_report_fields(&request.mode);
    let mut report = CompilerReport {
        schema: COMPILER_REPORT_SCHEMA,
        compiler_version: GRAPH_COMPILER_VERSION,
        status: CompilerStatus::Refused,
        mode: mode_name.to_owned(),
        target: request.target,
        topology_id: topology.topology_id.clone(),
        topology_content_hash: topology_hash.clone(),
        case_space_id: request.case_space_id.clone(),
        base_revision_id: request.base_revision_id.clone(),
        reviewed_claim_cell_id: reviewed_claim,
        accepted_review_id: accepted_review,
        generated_plan_review_status: "unreviewed",
        unsupported_semantics: Vec::new(),
        information_loss: Vec::new(),
    };

    let verification_policies = request
        .verification_policies
        .values()
        .filter_map(|document| {
            crate::verification_policy::parse_verification_policy(&document.to_string()).ok()
        })
        .map(|policy| (policy.verification_policy_id.clone(), policy))
        .collect::<BTreeMap<_, _>>();
    let analysis =
        lint_execution_topology_with_verification_policies(topology, &verification_policies);
    for finding in analysis
        .findings
        .iter()
        .filter(|finding| finding.is_deterministic_error())
    {
        report.unsupported_semantics.push(compiler_finding(
            format!("lint_{}", finding.code),
            finding.location.clone(),
            finding.detail.clone(),
        ));
    }
    if topology.case_space_id != request.case_space_id {
        report.unsupported_semantics.push(compiler_finding(
            "case_space_mismatch",
            "$.case_space_id",
            "topology and compiler request must name the same case space",
        ));
    }
    required_request_field(&request.case_space_id, "$.case_space_id", &mut report);
    required_request_field(&request.base_revision_id, "$.base_revision_id", &mut report);
    required_request_field(&request.plan_id, "$.plan_id", &mut report);
    validate_reviewed_binding(request, &topology_hash, &mut report);
    validate_reviewed_policy_binding(topology, request, &topology_hash, &mut report);
    validate_policy_documents(topology, request, &mut report);
    let mappings = validate_plan_mappings(topology, request, &mut report);

    if !report.unsupported_semantics.is_empty() || !report.information_loss.is_empty() {
        sort_report(&mut report);
        return Err(Box::new(report));
    }

    let plan = build_execution_plan(topology, request, &mappings).map_err(|finding| {
        report.unsupported_semantics.push(finding);
        sort_report(&mut report);
        Box::new(report.clone())
    })?;
    debug_assert_eq!(plan.review_status, ReviewStatus::Unreviewed);

    let mut artifact_values = build_artifact_values(topology, request, &analysis, &plan, &report);
    artifact_values.insert(
        "compiler.inputs.json",
        serde_json::to_value(retained_compiler_inputs(request)).expect("compiler inputs serialize"),
    );
    let mut artifacts = Vec::new();
    for (path, value) in artifact_values {
        let bytes = canonical_json_bytes(&value).expect("compiler-owned JSON serializes");
        artifacts.push(bundle_artifact(path, bytes));
    }
    artifacts.push(bundle_artifact(
        "execution.topology.json",
        canonical_execution_topology(topology)
            .expect("typed topology canonicalizes")
            .into_bytes(),
    ));
    artifacts.push(bundle_artifact(
        "topology.content-hash",
        format!("{topology_hash}\n").into_bytes(),
    ));
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));

    report.status = CompilerStatus::Compiled;
    replace_compiler_report(&mut artifacts, &report);
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = BundleManifest {
        schema: DEPLOYMENT_BUNDLE_SCHEMA.to_owned(),
        compiler_version: GRAPH_COMPILER_VERSION.to_owned(),
        topology_id: topology.topology_id.clone(),
        topology_content_hash: topology_hash,
        case_space_id: request.case_space_id.clone(),
        base_revision_id: request.base_revision_id.clone(),
        mode: mode_name.to_owned(),
        policy_manifest_content_hash: deployment_policy_manifest_content_hash(
            &deployment_policy_manifest(
                topology,
                &report.topology_content_hash,
                &request.verification_policies,
                &request.budget_policies,
                &request.expansion_policies,
            ),
        )
        .expect("typed deployment policy manifest hashes"),
        reviewed_claim_cell_id: report.reviewed_claim_cell_id.clone(),
        accepted_review_id: report.accepted_review_id.clone(),
        accepted_review_revision_id: accepted_review_revision,
        artifacts: artifacts
            .iter()
            .map(|artifact| BundleManifestEntry {
                path: artifact.path.clone(),
                content_hash: artifact.content_hash.clone(),
                byte_length: artifact.bytes.len() as u64,
            })
            .collect(),
    };
    let manifest_bytes = canonical_json_bytes(&manifest).expect("manifest serializes");
    let manifest_content_hash = crate::native_hash::sha256_hex(&manifest_bytes);
    Ok(DeploymentBundle {
        artifacts,
        manifest,
        manifest_bytes,
        manifest_content_hash,
    })
}

fn retained_compiler_inputs(request: &CompilerRequest) -> CompilerInputsArtifact {
    let mode = match &request.mode {
        CompilationMode::Proposal => RetainedCompilationMode::Proposal,
        CompilationMode::Reviewed(binding) => RetainedCompilationMode::Reviewed {
            claim_cell_id: binding.claim_cell_id.clone(),
            review_id: binding.review_id.clone(),
            topology_content_hash: binding.topology_content_hash.clone(),
            policy_manifest_content_hash: binding.policy_manifest_content_hash.clone(),
            case_space_id: binding.case_space_id.clone(),
            base_revision_id: binding.base_revision_id.clone(),
            expansion_proposal_id: binding.expansion_proposal_id.clone(),
        },
    };
    let mut node_plan_mappings = request.node_plan_mappings.clone();
    node_plan_mappings.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    CompilerInputsArtifact {
        schema: COMPILER_INPUTS_SCHEMA.to_owned(),
        compiler_version: GRAPH_COMPILER_VERSION.to_owned(),
        mode,
        target: request.target,
        case_space_id: request.case_space_id.clone(),
        base_revision_id: request.base_revision_id.clone(),
        plan_id: request.plan_id.clone(),
        node_plan_mappings,
        verification_policies: request.verification_policies.clone(),
        budget_policies: request.budget_policies.clone(),
        expansion_policies: request.expansion_policies.clone(),
    }
}

fn validate_reviewed_policy_binding(
    topology: &ExecutionTopology,
    request: &CompilerRequest,
    topology_hash: &str,
    report: &mut CompilerReport,
) {
    let manifest = deployment_policy_manifest(
        topology,
        topology_hash,
        &request.verification_policies,
        &request.budget_policies,
        &request.expansion_policies,
    );
    for finding in validate_deployment_policy_manifest(topology, topology_hash, &manifest) {
        report.unsupported_semantics.push(compiler_finding(
            finding.code,
            finding.location,
            finding.detail,
        ));
    }
    let CompilationMode::Reviewed(binding) = &request.mode else {
        return;
    };
    let manifest_hash = deployment_policy_manifest_content_hash(&manifest)
        .expect("compiler-owned policy manifest serializes deterministically");
    if manifest_hash != binding.policy_manifest_content_hash {
        report.unsupported_semantics.push(compiler_finding(
            "reviewed_policy_manifest_hash_mismatch",
            "$.mode.policy_manifest_content_hash",
            "policy documents no longer match the manifest accepted with the topology review",
        ));
    }
}

fn validate_reviewed_binding(
    request: &CompilerRequest,
    topology_hash: &str,
    report: &mut CompilerReport,
) {
    let CompilationMode::Reviewed(binding) = &request.mode else {
        return;
    };
    for (matches, code, location, detail) in [
        (
            binding.topology_content_hash == topology_hash,
            "reviewed_topology_hash_mismatch",
            "$.mode.topology_content_hash",
            "topology bytes no longer match the reviewed hash",
        ),
        (
            binding.case_space_id == request.case_space_id,
            "reviewed_case_space_mismatch",
            "$.case_space_id",
            "request case space differs from the reviewed binding",
        ),
        (
            binding.base_revision_id == request.base_revision_id,
            "stale_reviewed_base_revision",
            "$.base_revision_id",
            "request must name the exact revision observed by the accepted review",
        ),
    ] {
        if !matches {
            report
                .unsupported_semantics
                .push(compiler_finding(code, location, detail));
        }
    }
}

fn validate_policy_documents(
    topology: &ExecutionTopology,
    request: &CompilerRequest,
    report: &mut CompilerReport,
) {
    for (kind, expected, supplied) in [
        (
            "verification",
            &topology.verification_policy_ids,
            &request.verification_policies,
        ),
        (
            "budget",
            &topology.budget_policy_ids,
            &request.budget_policies,
        ),
        (
            "expansion",
            &topology.expansion_policy_ids,
            &request.expansion_policies,
        ),
    ] {
        let expected = expected.iter().cloned().collect::<BTreeSet<_>>();
        let supplied = supplied.keys().cloned().collect::<BTreeSet<_>>();
        for missing in expected.difference(&supplied) {
            report.information_loss.push(CompilerInformationLoss {
                code: format!("missing_{kind}_policy"),
                severity: if kind == "budget" {
                    InformationLossSeverity::SafetyAffecting
                } else {
                    InformationLossSeverity::AcceptanceAffecting
                },
                source_ids: vec![missing.clone()],
                detail: format!("target bundle cannot preserve undeclared {kind} policy {missing}"),
            });
        }
        for policy_id in expected.intersection(&supplied) {
            let document = &supplied_policy_documents(kind, request)[policy_id];
            if let Err(detail) = validate_policy_document(kind, policy_id, document) {
                report.information_loss.push(CompilerInformationLoss {
                    code: format!("invalid_{kind}_policy_document"),
                    severity: if kind == "budget" {
                        InformationLossSeverity::SafetyAffecting
                    } else {
                        InformationLossSeverity::AcceptanceAffecting
                    },
                    source_ids: vec![policy_id.clone()],
                    detail,
                });
            }
        }
        for extra in supplied.difference(&expected) {
            report.unsupported_semantics.push(compiler_finding(
                format!("undeclared_{kind}_policy"),
                format!("$.{kind}_policies"),
                format!("supplied policy {extra} is not declared by the topology"),
            ));
        }
    }
}

fn validate_policy_document(kind: &str, policy_id: &str, document: &Value) -> Result<(), String> {
    match kind {
        "verification" => {
            let source = serde_json::to_string(document)
                .map_err(|error| format!("verification policy does not serialize: {error}"))?;
            let policy = parse_verification_policy(&source).map_err(|findings| {
                format!(
                    "verification policy failed its canonical validator: {}",
                    findings
                        .into_iter()
                        .map(|finding| format!("{}: {}", finding.code, finding.detail))
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            })?;
            if policy.verification_policy_id != policy_id {
                return Err(
                    "verification_policy_id must exactly match its topology reference".to_owned(),
                );
            }
            Ok(())
        }
        "expansion" => {
            let policy: ExpansionPolicy = serde_json::from_value(document.clone())
                .map_err(|error| format!("expansion policy has an invalid shape: {error}"))?;
            let findings = validate_expansion_policy(&policy);
            if !findings.is_empty() {
                return Err(format!(
                    "expansion policy failed its canonical validator: {}",
                    findings
                        .into_iter()
                        .map(|finding| format!("{}: {}", finding.code, finding.detail))
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
            if policy.expansion_policy_id != policy_id {
                return Err(
                    "expansion_policy_id must exactly match its topology reference".to_owned(),
                );
            }
            Ok(())
        }
        // Budget policies do not yet have a repository-owned typed contract.
        // Keep the generic identity explicit instead of pretending to validate
        // semantics that CaseGraphen has not defined.
        "budget" => {
            if document
                .as_object()
                .and_then(|object| object.get("policy_id"))
                != Some(&Value::String(policy_id.to_owned()))
            {
                return Err(
                    "budget policy must be an object whose policy_id exactly matches its topology reference"
                        .to_owned(),
                );
            }
            Ok(())
        }
        _ => unreachable!("compiler owns policy kinds"),
    }
}

fn supplied_policy_documents<'a>(
    kind: &str,
    request: &'a CompilerRequest,
) -> &'a BTreeMap<String, Value> {
    match kind {
        "verification" => &request.verification_policies,
        "budget" => &request.budget_policies,
        "expansion" => &request.expansion_policies,
        _ => unreachable!("compiler owns policy kinds"),
    }
}

fn validate_plan_mappings<'a>(
    topology: &ExecutionTopology,
    request: &'a CompilerRequest,
    report: &mut CompilerReport,
) -> BTreeMap<&'a str, &'a NodePlanMapping> {
    let node_ids = topology
        .nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut mappings = BTreeMap::new();
    for mapping in &request.node_plan_mappings {
        if !node_ids.contains(mapping.node_id.as_str()) {
            report.unsupported_semantics.push(compiler_finding(
                "unknown_plan_mapping_node",
                "$.node_plan_mappings",
                format!("{} is not a topology node", mapping.node_id),
            ));
        }
        if mappings.insert(mapping.node_id.as_str(), mapping).is_some() {
            report.unsupported_semantics.push(compiler_finding(
                "duplicate_plan_mapping",
                "$.node_plan_mappings",
                format!("{} has more than one plan mapping", mapping.node_id),
            ));
        }
        if mapping.worker_binding_id.trim().is_empty()
            || mapping.success_evidence_requirement_ids.is_empty()
            || mapping.allowed_transition_classes.is_empty()
        {
            report.information_loss.push(CompilerInformationLoss {
                code: "incomplete_execution_plan_mapping".to_owned(),
                severity: InformationLossSeverity::AcceptanceAffecting,
                source_ids: vec![mapping.node_id.clone()],
                detail: "worker binding, success evidence, and allowed transitions are required"
                    .to_owned(),
            });
        }
    }
    for missing in node_ids
        .iter()
        .filter(|node_id| !mappings.contains_key(**node_id))
    {
        report.information_loss.push(CompilerInformationLoss {
            code: "missing_execution_plan_mapping".to_owned(),
            severity: InformationLossSeverity::AcceptanceAffecting,
            source_ids: vec![(*missing).to_owned()],
            detail: "target cannot lower a node without an explicit CaseGraphen plan mapping"
                .to_owned(),
        });
    }
    mappings
}

fn build_execution_plan(
    topology: &ExecutionTopology,
    request: &CompilerRequest,
    mappings: &BTreeMap<&str, &NodePlanMapping>,
) -> Result<ExecutionPlan, CompilerFinding> {
    let mut nodes = topology.nodes.iter().collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    let value = json!({
        "schema": crate::exec::EXECUTION_PLAN_SCHEMA,
        "schema_version": crate::exec::EXECUTION_PLAN_SCHEMA_VERSION,
        "plan_id": request.plan_id,
        "case_space_id": request.case_space_id,
        "base_revision_id": request.base_revision_id,
        "steps": nodes.into_iter().map(|node| {
            let mapping = mappings[node.node_id.as_str()];
            json!({
                "step_id": node.node_id,
                "work_cell_id": node.work_cell_id,
                "worker_binding_id": mapping.worker_binding_id,
                "success_evidence_requirement_ids": mapping.success_evidence_requirement_ids,
                "allowed_transition_classes": mapping.allowed_transition_classes,
            })
        }).collect::<Vec<_>>(),
        "provenance": {
            "source": {"kind": "code", "title": "CaseGraphen graph compiler proposal"},
            "confidence": 1.0,
            "review_status": "unreviewed",
            "extraction_method": GRAPH_COMPILER_VERSION
        },
        "review_status": "unreviewed",
        "metadata": {}
    });
    let plan: ExecutionPlan = serde_json::from_value(value).map_err(|error| {
        compiler_finding(
            "invalid_generated_execution_plan",
            "$.execution_plan",
            error.to_string(),
        )
    })?;
    validate_execution_plan(&plan).map_err(|error| {
        compiler_finding(
            "invalid_generated_execution_plan",
            "$.execution_plan",
            error.to_string(),
        )
    })?;
    if plan.review_status != ReviewStatus::Unreviewed
        || plan.provenance.review_status != ReviewStatus::Unreviewed
    {
        return Err(compiler_finding(
            "generated_plan_not_unreviewed",
            "$.execution_plan.review_status",
            "compiler output must remain unreviewed",
        ));
    }
    Ok(plan)
}

fn build_artifact_values(
    topology: &ExecutionTopology,
    request: &CompilerRequest,
    analysis: &GraphLintReport,
    plan: &ExecutionPlan,
    report: &CompilerReport,
) -> BTreeMap<&'static str, Value> {
    let topology_hash = &report.topology_content_hash;
    let nodes = topology
        .nodes
        .iter()
        .map(|node| {
            json!({
                "node_id": node.node_id,
                "executor_class": node.executor_class,
                "delivery": node.delivery,
                "idempotency_key": node.idempotency_key,
                "verification_policy_id": node.verification_policy_id,
                "budget_policy_id": node.budget_policy_id,
                "expansion_policy_id": node.expansion_policy_id,
            })
        })
        .collect::<Vec<_>>();
    let mut values = BTreeMap::new();
    values.insert(
        "graph.analysis.report.json",
        serde_json::to_value(analysis).expect("analysis serializes"),
    );
    values.insert(
        "case.mapping.genesis.proposal.json",
        json!({
            "schema": "casegraphen.experimental.case_mapping.proposal.v0",
            "review_status": "unreviewed",
            "case_space_id": request.case_space_id,
            "base_revision_id": request.base_revision_id,
            "topology_id": topology.topology_id,
            "topology_content_hash": topology_hash,
            "nodes": topology.nodes.iter().map(|node| json!({"node_id":node.node_id,"work_cell_id":node.work_cell_id})).collect::<Vec<_>>()
        }),
    );
    values.insert(
        "execution.plan.proposal.json",
        serde_json::to_value(plan).expect("plan serializes"),
    );
    values.insert(
        "runtime.deployment.json",
        json!({
            "schema": "casegraphen.experimental.runtime.deployment.generic_jsonl.v0",
            "target": request.target,
            "protocol": "generic_jsonl",
            "topology_id": topology.topology_id,
            "topology_content_hash": topology_hash,
            "case_space_id": request.case_space_id,
            "base_revision_id": request.base_revision_id,
            "nodes": nodes,
            "edges": topology.edges,
            "resource_requirements_preserved_by_manifest": true,
            "acceptance_authority": "none; runtime reports remain untrusted"
        }),
    );
    values.insert(
        "verification.policies.json",
        policy_value(
            "verification",
            topology_hash,
            &request.verification_policies,
        ),
    );
    values.insert(
        "budget.policies.json",
        policy_value("budget", topology_hash, &request.budget_policies),
    );
    values.insert(
        "expansion.policies.json",
        policy_value("expansion", topology_hash, &request.expansion_policies),
    );
    values.insert(
        "resource.manifest.json",
        json!({
            "schema":"casegraphen.experimental.resource_manifest.v0",
            "topology_id":topology.topology_id,
            "topology_content_hash":topology_hash,
            "nodes":topology.nodes.iter().map(|node|json!({"node_id":node.node_id,"resource_claims":node.resource_claims})).collect::<Vec<_>>()
        }),
    );
    values.insert(
        "compiler.report.json",
        serde_json::to_value(report).expect("report serializes"),
    );
    values
}

fn policy_value(kind: &str, topology_hash: &str, policies: &BTreeMap<String, Value>) -> Value {
    json!({
        "schema": format!("casegraphen.experimental.{kind}_policies.v0"),
        "topology_content_hash": topology_hash,
        "policies": policies
    })
}

fn replace_compiler_report(artifacts: &mut Vec<BundleArtifact>, report: &CompilerReport) {
    artifacts.retain(|artifact| artifact.path != "compiler.report.json");
    artifacts.push(bundle_artifact(
        "compiler.report.json",
        canonical_json_bytes(report).expect("report serializes"),
    ));
}

fn bundle_artifact(path: impl Into<String>, bytes: Vec<u8>) -> BundleArtifact {
    BundleArtifact {
        path: path.into(),
        content_hash: crate::native_hash::sha256_hex(&bytes),
        bytes,
    }
}

fn canonical_json_bytes(value: &impl Serialize) -> Result<Vec<u8>, serde_json::Error> {
    let value = canonical_value(serde_json::to_value(value)?);
    serde_json::to_vec(&value)
}

fn canonical_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonical_value(value)))
                .collect(),
        ),
        scalar => scalar,
    }
}

fn mode_report_fields(
    mode: &CompilationMode,
) -> (&'static str, Option<String>, Option<String>, Option<String>) {
    match mode {
        CompilationMode::Proposal => ("proposal", None, None, None),
        CompilationMode::Reviewed(binding) => (
            "reviewed",
            Some(binding.claim_cell_id.clone()),
            Some(binding.review_id.clone()),
            Some(binding.base_revision_id.clone()),
        ),
    }
}

fn required_request_field(value: &str, location: &str, report: &mut CompilerReport) {
    if value.trim().is_empty() {
        report.unsupported_semantics.push(compiler_finding(
            "empty_required_field",
            location,
            "value must not be empty",
        ));
    }
}

fn compiler_finding(
    code: impl Into<String>,
    location: impl Into<String>,
    detail: impl Into<String>,
) -> CompilerFinding {
    CompilerFinding {
        code: code.into(),
        location: location.into(),
        detail: detail.into(),
    }
}

fn sort_report(report: &mut CompilerReport) {
    report.unsupported_semantics.sort_by(|left, right| {
        (&left.code, &left.location, &left.detail).cmp(&(
            &right.code,
            &right.location,
            &right.detail,
        ))
    });
    report.information_loss.sort_by(|left, right| {
        (&left.code, &left.source_ids, &left.detail).cmp(&(
            &right.code,
            &right.source_ids,
            &right.detail,
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_model::{CaseCellLifecycle, CaseCellType, CaseMorphismType};

    fn topology() -> ExecutionTopology {
        crate::execution_topology::parse_execution_topology(include_str!(
            "../schemas/experimental/execution.topology.file-review.example.json"
        ))
        .expect("topology example")
    }

    fn request(topology: &ExecutionTopology) -> CompilerRequest {
        let transition = AllowedTransitionClass {
            morphism_type: CaseMorphismType::Update,
            target_cell_types: vec![CaseCellType::Work],
            to_lifecycles: vec![CaseCellLifecycle::Resolved],
        };
        CompilerRequest {
            mode: CompilationMode::Proposal,
            target: CompilationTarget::GenericJsonlV0,
            case_space_id: topology.case_space_id.clone(),
            base_revision_id: "revision:explicit-base".to_owned(),
            plan_id: "plan:compiled-topology".to_owned(),
            node_plan_mappings: topology
                .nodes
                .iter()
                .map(|node| NodePlanMapping {
                    node_id: node.node_id.clone(),
                    worker_binding_id: format!("worker_binding:{}", node.node_id),
                    success_evidence_requirement_ids: vec![format!(
                        "evidence_requirement:{}",
                        node.node_id
                    )],
                    allowed_transition_classes: vec![transition.clone()],
                })
                .collect(),
            verification_policies: topology
                .verification_policy_ids
                .iter()
                .map(|id| {
                    let mut policy: Value = serde_json::from_str(include_str!(
                        "../schemas/experimental/verification.policy.example.json"
                    ))
                    .expect("verification policy example");
                    policy["verification_policy_id"] = Value::String(id.clone());
                    (id.clone(), policy)
                })
                .collect(),
            budget_policies: topology
                .budget_policy_ids
                .iter()
                .map(|id| (id.clone(), json!({"policy_id":id,"max_cost":10})))
                .collect(),
            expansion_policies: topology
                .expansion_policy_ids
                .iter()
                .map(|id| {
                    let mut policy: Value = serde_json::from_str(include_str!(
                        "../schemas/experimental/expansion.policy.example.json"
                    ))
                    .expect("expansion policy example");
                    policy["expansion_policy_id"] = Value::String(id.clone());
                    (id.clone(), policy)
                })
                .collect(),
        }
    }

    #[test]
    fn generic_jsonl_proposal_compiles_to_a_deterministic_addressed_bundle() {
        let mut left = topology();
        let first_request = request(&left);
        let first = compile_execution_topology(&left, &first_request).expect("compile proposal");
        left.nodes.reverse();
        left.edges.reverse();
        let mut reordered_request = request(&left);
        reordered_request.node_plan_mappings.reverse();
        let second =
            compile_execution_topology(&left, &reordered_request).expect("compile reordered");

        assert_eq!(first, second);
        assert_eq!(first.manifest.mode, "proposal");
        assert_eq!(
            first.manifest_content_hash,
            crate::native_hash::sha256_hex(&first.manifest_bytes)
        );
        let paths = first
            .artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect::<BTreeSet<_>>();
        let expected: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/graph-compiler/generic-jsonl.expected.json"
        ))
        .unwrap();
        for required in expected["required_artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
        {
            assert!(paths.contains(required), "missing {required}");
        }
        for artifact in &first.artifacts {
            assert_eq!(
                artifact.content_hash,
                crate::native_hash::sha256_hex(&artifact.bytes)
            );
            assert!(first.manifest.artifacts.iter().any(|entry| {
                entry.path == artifact.path
                    && entry.content_hash == artifact.content_hash
                    && entry.byte_length == artifact.bytes.len() as u64
            }));
        }
        let topology_artifact = first
            .artifacts
            .iter()
            .find(|artifact| artifact.path == "execution.topology.json")
            .unwrap();
        assert_eq!(
            topology_artifact.content_hash,
            first.manifest.topology_content_hash
        );
        let plan_artifact = first
            .artifacts
            .iter()
            .find(|artifact| artifact.path == "execution.plan.proposal.json")
            .unwrap();
        let plan: ExecutionPlan = serde_json::from_slice(&plan_artifact.bytes).unwrap();
        assert_eq!(plan.review_status, ReviewStatus::Unreviewed);
        assert_eq!(plan.provenance.review_status, ReviewStatus::Unreviewed);
    }

    #[test]
    fn deployment_authority_requires_verified_manifest_and_artifact_bytes() {
        let topology = topology();
        let request = request(&topology);
        let bundle = compile_execution_topology(&topology, &request).expect("compile proposal");
        let expected_hash = bundle.manifest_content_hash.clone();
        let verified = verify_deployment_bundle(bundle.clone(), &expected_hash)
            .expect("exact compiler output verifies");
        assert_eq!(verified.manifest_content_hash(), expected_hash);

        let forged = "0".repeat(64);
        assert_eq!(
            verify_deployment_bundle(bundle.clone(), &forged)
                .expect_err("a caller-chosen digest cannot mint bundle authority")
                .code,
            "deployment_bundle_integrity_failure"
        );

        let mut substituted = bundle;
        substituted
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.path == "execution.topology.json")
            .unwrap()
            .bytes
            .push(b' ');
        assert_eq!(
            verify_deployment_bundle(substituted, &expected_hash)
                .expect_err("artifact substitution must fail before authority derivation")
                .code,
            "deployment_bundle_integrity_failure"
        );
    }

    fn readdress_artifact_after_substitution(bundle: &mut DeploymentBundle, path: &str) {
        let artifact = bundle
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.path == path)
            .expect("test artifact exists");
        artifact.bytes.push(b' ');
        artifact.content_hash = crate::native_hash::sha256_hex(&artifact.bytes);
        let entry = bundle
            .manifest
            .artifacts
            .iter_mut()
            .find(|entry| entry.path == path)
            .expect("manifest entry exists");
        entry.content_hash = artifact.content_hash.clone();
        entry.byte_length = artifact.bytes.len() as u64;
        bundle.manifest_bytes =
            canonical_json_bytes(&bundle.manifest).expect("manifest serializes");
        bundle.manifest_content_hash = crate::native_hash::sha256_hex(&bundle.manifest_bytes);
    }

    #[test]
    fn self_consistent_artifact_substitution_cannot_mint_compiler_provenance() {
        let topology = topology();
        let request = request(&topology);
        let original = compile_execution_topology(&topology, &request).expect("compile proposal");
        let paths = original
            .artifacts
            .iter()
            .map(|artifact| artifact.path.clone())
            .collect::<Vec<_>>();

        for path in paths {
            let mut substituted = original.clone();
            readdress_artifact_after_substitution(&mut substituted, &path);
            let substituted_hash = substituted.manifest_content_hash.clone();
            let finding = verify_deployment_bundle(substituted, &substituted_hash)
                .expect_err("content-addressed substitution is not compiler provenance");
            assert_eq!(
                finding.code, "deployment_bundle_semantic_mismatch",
                "unexpected finding for {path}: {finding:?}"
            );
        }
    }

    #[test]
    fn missing_policy_or_plan_mapping_is_explicit_information_loss_and_refuses() {
        let topology = topology();
        let mut request = request(&topology);
        request.verification_policies.clear();
        request.node_plan_mappings.pop();
        let report = compile_execution_topology(&topology, &request).unwrap_err();
        assert_eq!(report.status, CompilerStatus::Refused);
        assert!(report
            .information_loss
            .iter()
            .any(|loss| loss.code == "missing_verification_policy"
                && loss.severity == InformationLossSeverity::AcceptanceAffecting));
        assert!(report
            .information_loss
            .iter()
            .any(|loss| loss.code == "missing_execution_plan_mapping"));
    }

    #[test]
    fn malformed_policy_document_cannot_be_claimed_as_preserved() {
        let topology = topology();
        let mut request = request(&topology);
        request.verification_policies.insert(
            "verification:independent".to_owned(),
            json!({"verification_policy_id":"verification:different"}),
        );
        let report = compile_execution_topology(&topology, &request).unwrap_err();
        assert!(report
            .information_loss
            .iter()
            .any(|loss| loss.code == "invalid_verification_policy_document"));
    }

    #[test]
    fn compiler_consumes_the_real_verification_and_expansion_policy_contracts() {
        let mut topology = topology();
        let verification_id = "verification:security-finding".to_owned();
        topology.verification_policy_ids = vec![verification_id.clone()];
        for node in &mut topology.nodes {
            if node.verification_policy_id.is_some() {
                node.verification_policy_id = Some(verification_id.clone());
            }
        }
        let expansion_id = "expansion:bug-discovery".to_owned();
        topology.expansion_policy_ids = vec![expansion_id.clone()];
        topology.nodes[0].expansion_policy_id = Some(expansion_id.clone());

        let mut request = request(&topology);
        let verification: Value = serde_json::from_str(include_str!(
            "../schemas/experimental/verification.policy.example.json"
        ))
        .expect("real verification policy example");
        let expansion: Value = serde_json::from_str(include_str!(
            "../schemas/experimental/expansion.policy.example.json"
        ))
        .expect("real expansion policy example");
        request.verification_policies = BTreeMap::from([(verification_id, verification.clone())]);
        request.expansion_policies = BTreeMap::from([(expansion_id, expansion.clone())]);

        let bundle = compile_execution_topology(&topology, &request)
            .expect("real repository-owned policy contracts compile");
        let verification_artifact = bundle
            .artifacts
            .iter()
            .find(|artifact| artifact.path == "verification.policies.json")
            .expect("verification policy artifact");
        let expansion_artifact = bundle
            .artifacts
            .iter()
            .find(|artifact| artifact.path == "expansion.policies.json")
            .expect("expansion policy artifact");
        let verification_bundle: Value =
            serde_json::from_slice(&verification_artifact.bytes).expect("verification bundle");
        let expansion_bundle: Value =
            serde_json::from_slice(&expansion_artifact.bytes).expect("expansion bundle");
        assert!(verification_bundle["policies"]
            .as_object()
            .expect("verification policy map")
            .values()
            .any(|value| value == &verification));
        assert!(expansion_bundle["policies"]
            .as_object()
            .expect("expansion policy map")
            .values()
            .any(|value| value == &expansion));
    }

    #[test]
    fn reviewed_mode_is_distinct_and_a_topology_edit_invalidates_its_binding() {
        let mut topology = topology();
        let topology_hash = execution_topology_content_hash(&topology).unwrap();
        let mut request = request(&topology);
        let policy_manifest_content_hash = request_policy_manifest_hash(&topology, &request);
        request.mode = CompilationMode::Reviewed(ReviewedTopologyBinding {
            claim_cell_id: "evidence:reviewed-topology".to_owned(),
            review_id: "review:accepted-topology".to_owned(),
            topology_content_hash: topology_hash,
            policy_manifest_content_hash,
            case_space_id: request.case_space_id.clone(),
            base_revision_id: request.base_revision_id.clone(),
            expansion_proposal_id: None,
        });
        let reviewed = compile_execution_topology(&topology, &request).expect("reviewed bundle");
        assert_eq!(reviewed.manifest.mode, "reviewed");

        request
            .budget_policies
            .get_mut("budget:small")
            .expect("fixture budget policy")["max_cost"] = json!(999);
        let substituted = compile_execution_topology(&topology, &request).unwrap_err();
        assert!(substituted
            .unsupported_semantics
            .iter()
            .any(|finding| finding.code == "reviewed_policy_manifest_hash_mismatch"));

        topology.nodes[0].purpose.push_str(" changed");
        let report = compile_execution_topology(&topology, &request).unwrap_err();
        assert!(report
            .unsupported_semantics
            .iter()
            .any(|finding| finding.code == "reviewed_topology_hash_mismatch"));
    }

    #[test]
    fn canonical_review_binding_can_name_one_exact_expansion_proposal() {
        let mut reviewed_topology = topology();
        reviewed_topology.nodes[0].purpose.push_str(" expanded");
        let topology_hash = execution_topology_content_hash(&reviewed_topology).unwrap();
        let proposal_id = "proposal:sha256-reviewed-expansion";
        let request = request(&reviewed_topology);
        let policy_manifest_content_hash =
            request_policy_manifest_hash(&reviewed_topology, &request);
        let mode = CompilationMode::Reviewed(ReviewedTopologyBinding {
            claim_cell_id: "evidence:reviewed-expansion".to_owned(),
            review_id: "review:accepted-expansion".to_owned(),
            topology_content_hash: topology_hash,
            policy_manifest_content_hash,
            case_space_id: reviewed_topology.case_space_id.clone(),
            base_revision_id: "revision:accepted-expansion".to_owned(),
            expansion_proposal_id: Some(proposal_id.to_owned()),
        });
        assert!(crate::dynamic_expansion::accepted_expansion_review_binding(
            &mode,
            proposal_id,
            &reviewed_topology,
        )
        .is_ok());
        assert!(crate::dynamic_expansion::accepted_expansion_review_binding(
            &mode,
            "proposal:other",
            &reviewed_topology,
        )
        .is_err());
    }

    fn request_policy_manifest_hash(
        topology: &ExecutionTopology,
        request: &CompilerRequest,
    ) -> String {
        let topology_hash = execution_topology_content_hash(topology).unwrap();
        deployment_policy_manifest_content_hash(&deployment_policy_manifest(
            topology,
            &topology_hash,
            &request.verification_policies,
            &request.budget_policies,
            &request.expansion_policies,
        ))
        .unwrap()
    }

    #[test]
    fn deterministic_lint_errors_refuse_compilation() {
        let mut topology = topology();
        topology.edges[0].to = topology.edges[0].from.clone();
        let request = request(&topology);
        let report = compile_execution_topology(&topology, &request).unwrap_err();
        assert!(report
            .unsupported_semantics
            .iter()
            .any(|finding| finding.code.starts_with("lint_contract_")
                || finding.code == "lint_dependency_cycle"));
    }
}

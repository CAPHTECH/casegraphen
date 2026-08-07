use super::{
    append_validated_morphism, io::provenance, io::timestamp, require_current_revision,
    validated_mutation_gate, NativeMutationGateOptions, NativeReviewApplyOptions,
};
use crate::evidence_trust::EvidenceTrustBoundary;
use crate::{
    native_eval::evaluate_native_case,
    native_model::{
        is_artifact_cell, CaseCell, CaseCellLifecycle, CaseCellType, CaseMorphism,
        CaseMorphismType, CaseRelation, CaseRelationType, CaseSpace, MorphismPayload,
        RelationStrength, ReviewAction, ARTIFACT_CELL_TYPE,
    },
    native_review::{
        accept_review_morphism, reject_review_morphism, reopen_review_morphism,
        waive_review_morphism, NativeOperationGate, NativeReviewRequest, NativeReviewTargetKind,
    },
    native_store::NativeCaseStore,
    path_confinement::path_confined,
};
use higher_graphen_core::{Id, ReviewStatus, SourceKind};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use super::super::{path_helpers::path_segment, NativeCliError, NativeEvidenceAttachment};

pub(in crate::native_cli) fn review_apply(
    store: &Path,
    case_space_id: &Id,
    options: NativeReviewApplyOptions<'_>,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, options.base_revision_id)?;
    let target_kind = review_target_kind(&replay.case_space, options.target_id)?;
    let request = NativeReviewRequest {
        target_kind,
        target_id: options.target_id.clone(),
        action: options.action,
        reviewer_id: options.reviewer_id.clone(),
        reviewed_at: timestamp(),
        reason: options.reason.to_owned(),
        evidence_ids: options.evidence_ids.to_vec(),
        source_ids: vec![options.target_id.clone()],
        target_revision_id: generated_revision_id(&replay.case_space, "review", options.target_id)?,
    };
    let mut morphism = match options.action {
        ReviewAction::Accept => accept_review_morphism(&replay.case_space, request)?,
        ReviewAction::Reject => reject_review_morphism(&replay.case_space, request)?,
        ReviewAction::Reopen => reopen_review_morphism(&replay.case_space, request)?,
        ReviewAction::Waive => waive_review_morphism(&replay.case_space, request)?,
        ReviewAction::Defer | ReviewAction::Supersede => {
            return Err(NativeCliError::invalid("unsupported CLI review action"))
        }
    };
    let command = match options.action {
        ReviewAction::Accept => "casegraphen review accept",
        ReviewAction::Reject => "casegraphen review reject",
        ReviewAction::Reopen => "casegraphen review reopen",
        ReviewAction::Waive => "casegraphen review waive",
        ReviewAction::Defer | ReviewAction::Supersede => unreachable!("action checked above"),
    };
    let operation_gate =
        validated_mutation_gate(&replay.case_space, options.gate_options, "review")?;
    morphism.metadata.insert(
        "operation_gate".to_owned(),
        serde_json::to_value(&operation_gate)?,
    );
    // What an acceptance activates is wider than the target id the command
    // names: every coverage pair the target's attach morphism recorded goes
    // live at once. Read from the same derivation the decision reads, and
    // reported so the record shows what the reviewer's acceptance covered.
    let activated_coverage = match options.action {
        ReviewAction::Accept => crate::native_eval::recorded_coverage_targets(
            &replay.case_space,
            options.target_id.as_str(),
        ),
        _ => Vec::new(),
    };
    let mut result = append_validated_morphism(
        &store_api,
        &replay.case_space,
        morphism,
        Some(operation_gate.actor_id),
        command,
    )?;
    if let Some(object) = result["result"].as_object_mut() {
        object.insert(
            "activated_coverage".to_owned(),
            serde_json::to_value(&activated_coverage)?,
        );
    }
    Ok(result)
}

pub(in crate::native_cli) fn evidence_attach(
    store: &Path,
    case_space_id: &Id,
    base_revision_id: &Id,
    attachments: &[NativeEvidenceAttachment],
    gate_options: &NativeMutationGateOptions,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, base_revision_id)?;
    // Authorize before touching the inputs. Reading them first let an actor
    // holding no capability tell an existing file from a missing one, and tell
    // an existing cell id from an unknown one, through the refusal text alone.
    // Nothing durable was at risk, but the gate is cheaper than the reads and
    // there is no reason for it to run second.
    let operation_gate =
        validated_mutation_gate(&replay.case_space, gate_options, "evidence-attach")?;
    let prepared = prepare_evidence_attachments(&replay.case_space, attachments)?;
    append_evidence_attach_morphism(
        &store_api,
        &replay.case_space,
        prepared,
        operation_gate,
        "casegraphen evidence attach",
    )
}

/// The one place that turns prepared claim(s)+artifact(s) into the single
/// `EvidenceAttach` morphism and appends it. `evidence attach` reaches this
/// after preparing one attachment per `--input` group; `packet apply` reaches
/// it after preparing its one claim — same gate operation, same morphism
/// shape, same append, so a hardening pass here cannot land on only one of
/// them.
pub(super) fn append_evidence_attach_morphism(
    store_api: &NativeCaseStore,
    case_space: &CaseSpace,
    prepared: Vec<PreparedEvidenceAttachment>,
    operation_gate: NativeOperationGate,
    command: &str,
) -> Result<Value, NativeCliError> {
    let mut morphism = evidence_attach_morphism(case_space, &prepared)?;
    morphism.metadata.insert(
        "operation_gate".to_owned(),
        serde_json::to_value(&operation_gate)?,
    );
    append_validated_morphism(
        store_api,
        case_space,
        morphism,
        Some(operation_gate.actor_id),
        command,
    )
}

fn evidence_attach_morphism(
    case_space: &CaseSpace,
    prepared: &[PreparedEvidenceAttachment],
) -> Result<CaseMorphism, NativeCliError> {
    let first_cell = &prepared
        .first()
        .ok_or_else(|| NativeCliError::usage("at least one evidence claim is required"))?
        .cell;
    let sequence = case_space.morphism_log.len() + 1;
    let cells = prepared
        .iter()
        .flat_map(|attachment| {
            std::iter::once(attachment.cell.clone()).chain(attachment.artifacts.iter().cloned())
        })
        .collect::<Vec<_>>();
    let relations = prepared
        .iter()
        .flat_map(|attachment| attachment.relations.iter().cloned())
        .collect::<Vec<_>>();
    let added_ids = prepared
        .iter()
        .flat_map(|attachment| {
            std::iter::once(attachment.cell.id.clone())
                .chain(attachment.artifacts.iter().map(|cell| cell.id.clone()))
                .chain(
                    attachment
                        .relations
                        .iter()
                        .map(|relation| relation.id.clone()),
                )
        })
        .collect();
    // The satisfies targets a claim named, read back from the relations
    // already minted for it rather than from a second copy of the caller's
    // input — the relation *is* the record of what was named.
    let preserved_ids = prepared
        .iter()
        .flat_map(|attachment| {
            attachment
                .relations
                .iter()
                .filter(|relation| {
                    relation.relation_type == CaseRelationType::SatisfiesEvidenceRequirement
                })
                .map(|relation| relation.to_id.clone())
        })
        .collect();
    // Deliberately the claim cells only, not the artifacts they cite: this
    // field is what a caller reads as "the evidence this morphism produced",
    // and an artifact is the observation a claim is about, not itself a claim.
    let evidence_ids = prepared
        .iter()
        .map(|attachment| attachment.cell.id.clone())
        .collect();
    let source_ids = prepared
        .iter()
        .flat_map(|attachment| attachment.cell.source_ids.iter().cloned())
        .collect();
    let mut metadata = Map::new();
    metadata.insert(
        "payload".to_owned(),
        serde_json::to_value(MorphismPayload {
            added_cells: cells,
            added_relations: relations,
            ..MorphismPayload::default()
        })?,
    );
    Ok(CaseMorphism {
        morphism_id: generated_operation_id("morphism:evidence-attach", &first_cell.id, sequence)?,
        morphism_type: CaseMorphismType::EvidenceAttach,
        source_revision_id: Some(case_space.revision.revision_id.clone()),
        target_revision_id: generated_operation_id(
            "revision:evidence-attach",
            &first_cell.id,
            sequence,
        )?,
        added_ids,
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids,
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids,
        source_ids,
        metadata,
    })
}

pub(super) struct PreparedEvidenceAttachment {
    cell: CaseCell,
    relations: Vec<CaseRelation>,
    artifacts: Vec<CaseCell>,
}

impl PreparedEvidenceAttachment {
    pub(super) fn claim_cell_id(&self) -> &Id {
        &self.cell.id
    }

    pub(super) fn artifact_cell_ids(&self) -> impl Iterator<Item = &Id> {
        self.artifacts.iter().map(|cell| &cell.id)
    }
}

/// What a coverage claim may name, answered once.
///
/// The evaluator treats coverage recorded against a *work* cell as satisfying
/// every evidence and proof requirement that cell has, so an attach naming a
/// work cell discharged requirements no morphism ever named and no reviewer
/// ever saw — `--satisfies work:tag-release` plus one `review accept` cleared
/// a blocking hard requirement on an unrelated evidence cell. `run --step`
/// already restricted its own coverage targets to evidence cells
/// (`existing_requirement_ids`); this is that same rule, and the two must not
/// answer it differently.
pub(in crate::native_cli) fn is_coverage_target(case_space: &CaseSpace, target_id: &Id) -> bool {
    case_space
        .case_cells
        .iter()
        .any(|cell| cell.id == *target_id && cell.cell_type == CaseCellType::Evidence)
}

pub(super) fn existing_case_space_ids(case_space: &CaseSpace) -> BTreeSet<Id> {
    case_space
        .case_cells
        .iter()
        .map(|cell| cell.id.clone())
        .chain(
            case_space
                .case_relations
                .iter()
                .map(|relation| relation.id.clone()),
        )
        .collect()
}

fn prepare_evidence_attachments(
    case_space: &CaseSpace,
    attachments: &[NativeEvidenceAttachment],
) -> Result<Vec<PreparedEvidenceAttachment>, NativeCliError> {
    let existing_ids = existing_case_space_ids(case_space);
    let mut state = ClaimPreparationState::new(&existing_ids);
    attachments
        .iter()
        .map(|attachment| {
            let bytes = fs::read(&attachment.input)
                .map_err(|source| input_refusal(&attachment.input, source))?;
            let cell = evidence_cell_from_bytes(&bytes)
                .map_err(|error| input_refusal(&attachment.input, error))?;
            prepare_claim(
                case_space,
                &mut state,
                &attachment.input,
                cell,
                &attachment.satisfies_ids,
                &attachment.artifact_paths,
                // Unconfined: the operator typed `--artifact` themselves, the
                // same as every other input flag (issue #21).
                None,
            )
        })
        .collect()
}

/// The bookkeeping `prepare_claim` threads across every claim it prepares:
/// which ids already exist, which ids this batch has already claimed, and
/// which artifact ids this batch has already staged. Bundled into one value
/// instead of three parameters so the function stays under the arity clippy
/// enforces — these three are never meaningfully separable anyway, since
/// every call site threads all three together.
pub(super) struct ClaimPreparationState<'a> {
    existing_ids: &'a BTreeSet<Id>,
    claimed_ids: BTreeMap<Id, PathBuf>,
    staged_artifact_ids: BTreeSet<Id>,
}

impl<'a> ClaimPreparationState<'a> {
    pub(super) fn new(existing_ids: &'a BTreeSet<Id>) -> Self {
        Self {
            existing_ids,
            claimed_ids: BTreeMap::new(),
            staged_artifact_ids: BTreeSet::new(),
        }
    }
}

/// One claim's contribution to an `EvidenceAttach` morphism: satisfies-target
/// validation and relations, artifact resolution and `derives_from` relations.
/// `evidence attach` calls this once per `--input` group; `packet apply` calls
/// it exactly once for the packet's one claim. `label` names the input in
/// refusal messages — the `--input` path for attach, the packet path for a
/// packet.
///
/// This is also the one place that resolves an artifact path (issue #21): each
/// path in `artifact_paths` is canonicalized once, and that single canonical
/// value is what gets read and what `resolve_artifact` records as
/// `artifact_uri` — never the caller-supplied string, and never a second,
/// possibly different, canonicalization. A path that does not canonicalize is
/// refused with the ordinary input-refusal shape rather than falling back to
/// the original.
///
/// `confine_artifacts_within` is the asymmetry ADR 0015 already draws: a
/// packet's `claim.id` is attacker-controlled text (that is why
/// `packet_apply`'s `next_operations` emits structured values instead of a
/// command string), and a packet's `artifacts:` list has the same origin —
/// `artifact_paths` for a packet are the raw, unjoined `artifacts:` entries,
/// and this function joins them onto `confine_within` itself, so there is
/// exactly one join and it is the same directory the containment check uses.
/// `evidence attach --artifact` passes `None` and its `artifact_paths` are
/// exactly the operator-typed paths, not joined onto anything: the operator
/// typed those themselves, same as every other input flag, so they are not
/// confined.
///
/// Confined resolution is three stages, in order, and all three matter:
///
/// 1. **Lexical rejection**, on the caller-supplied string alone, before any
///    filesystem call. An entry that is absolute or contains a `..`
///    component is refused outright. `fs::canonicalize` only validates the
///    *final resolved* path, so without this stage a crafted entry can climb
///    from an arbitrary absolute directory up to `/` (any number of `..` gets
///    there) and back down through this packet's own real, known directory
///    to a genuinely in-root file — canonicalizing and passing containment
///    successfully *whenever the climbed-through directory happens to
///    exist*, and failing whenever it does not. That makes ordinary
///    dispatch success/failure — not just the refusal message — a
///    filesystem-existence oracle for arbitrary absolute directories, no
///    planted symlink required. Reproduced: `artifacts: ["/etc/../../.. \
///    (enough to reach /) .../<this-packet-dir-without-leading-slash>/a.txt"]`
///    dispatched successfully, and mutated the store, whenever `/etc` (or
///    any other probed directory) existed.
/// 2. **Canonicalization** of the entry joined onto `confine_within`.
/// 3. **Containment** (`path_confined`, `crate::path_confinement` — the one
///    shared implementation of this predicate; the GitHub evidence adapter's
///    `capture_manifest.v0` `artifact_path` confinement uses the same
///    function) of the canonical result inside `confine_within` — still
///    required after stage 1, because it is what
///    catches an in-tree symlink whose target leaves the directory; a
///    lexical check cannot see through a symlink.
///
/// All four confined failure modes — lexical rejection, canonicalization
/// failure, a resolved-but-outside-the-root result, and a resolved-and-
/// in-root path that cannot be read (a directory, a permission error, ...)
/// — refuse with the identical message, naming neither the io error nor any
/// resolved path: only the caller-supplied string is echoed back, because
/// the proposer already wrote it. The fourth mode is scoped strictly
/// in-root, so it does not extend the oracle to arbitrary absolute paths —
/// it only means "exists as a directory" and "does not exist" stay
/// indistinguishable for a name inside the packet's own directory, same as
/// the other three. Unconfined (`None`) keeps the ordinary, informative io
/// error for all failure modes including this one, because the operator
/// already knows what they typed.
pub(super) fn prepare_claim(
    case_space: &CaseSpace,
    state: &mut ClaimPreparationState<'_>,
    label: &Path,
    cell: CaseCell,
    satisfies_ids: &[Id],
    artifact_paths: &[PathBuf],
    confine_artifacts_within: Option<&Path>,
) -> Result<PreparedEvidenceAttachment, NativeCliError> {
    for target_id in satisfies_ids {
        if !is_coverage_target(case_space, target_id) {
            return Err(input_refusal(
                label,
                format!("satisfies target {target_id} is not an evidence cell in this case space"),
            ));
        }
    }
    claim_attachment_id(state.existing_ids, &mut state.claimed_ids, &cell.id, label)?;
    let mut relations = satisfies_ids
        .iter()
        .enumerate()
        .map(|(index, target_id)| {
            Ok(CaseRelation {
                id: Id::new(format!(
                    "relation:evidence:{}:{}",
                    path_segment(&cell.id),
                    index + 1
                ))?,
                relation_type: CaseRelationType::SatisfiesEvidenceRequirement,
                relation_strength: RelationStrength::Diagnostic,
                from_id: cell.id.clone(),
                to_id: target_id.clone(),
                evidence_ids: vec![cell.id.clone()],
                source_ids: cell.source_ids.clone(),
                provenance: cell.provenance.clone(),
                metadata: Map::new(),
            })
        })
        .collect::<Result<Vec<_>, NativeCliError>>()
        .map_err(|error| input_refusal(label, error))?;
    for relation in &relations {
        claim_attachment_id(
            state.existing_ids,
            &mut state.claimed_ids,
            &relation.id,
            label,
        )?;
    }
    let mut artifacts = Vec::new();
    for (index, artifact_path) in artifact_paths.iter().enumerate() {
        let canonical_artifact_path = match confine_artifacts_within {
            Some(confine_within) => {
                // Stage 1: lexical rejection, string-only, no filesystem
                // call. See this function's doc comment for why an absolute
                // entry or a `..` component must be refused here rather than
                // left to canonicalize-and-contain.
                if artifact_path.is_absolute()
                    || artifact_path
                        .components()
                        .any(|component| component == std::path::Component::ParentDir)
                {
                    return Err(input_refusal(artifact_path, CONFINEMENT_REFUSAL));
                }
                // Stages 2 and 3: canonicalize the entry joined onto the
                // confinement root, then require the result stay inside it —
                // this is what catches an in-tree symlink pointing out,
                // which stage 1 cannot see.
                fs::canonicalize(confine_within.join(artifact_path))
                    .ok()
                    .filter(|canonical| path_confined(canonical, confine_within))
                    .ok_or_else(|| input_refusal(artifact_path, CONFINEMENT_REFUSAL))?
            }
            None => fs::canonicalize(artifact_path)
                .map_err(|source| input_refusal(artifact_path, source))?,
        };
        // A fourth confined failure mode, distinct from the three above:
        // an entry that resolves *in-root* and passes containment but
        // cannot be read (a directory, a permission error, ...) would
        // otherwise report the raw io error here, splitting "exists as a
        // directory" from "does not exist" for any relative name inside
        // the packet's own directory. Scope is strictly in-root, so this
        // does not extend the confinement oracle to arbitrary absolute
        // paths, but it is the same class — fold it into the identical
        // confinement refusal whenever confinement applies. Unconfined
        // (`evidence attach --artifact`) keeps the raw io error: there is
        // no oracle to close there.
        let artifact_bytes = fs::read(&canonical_artifact_path).map_err(|source| {
            if confine_artifacts_within.is_some() {
                input_refusal(artifact_path, CONFINEMENT_REFUSAL)
            } else {
                input_refusal(artifact_path, source)
            }
        })?;
        let (artifact_cell, relation) = resolve_artifact(
            case_space,
            &mut state.staged_artifact_ids,
            &cell,
            &canonical_artifact_path,
            &artifact_bytes,
            index + 1,
        )
        .map_err(|error| input_refusal(artifact_path, error))?;
        if let Some(artifact_cell) = artifact_cell {
            claim_attachment_id(
                state.existing_ids,
                &mut state.claimed_ids,
                &artifact_cell.id,
                label,
            )?;
            artifacts.push(artifact_cell);
        }
        claim_attachment_id(
            state.existing_ids,
            &mut state.claimed_ids,
            &relation.id,
            label,
        )?;
        relations.push(relation);
    }
    Ok(PreparedEvidenceAttachment {
        cell,
        relations,
        artifacts,
    })
}

/// The one confined-artifact refusal message, shared verbatim by all four
/// confined failure modes (lexical rejection, canonicalization failure,
/// resolved-but-outside-the-root, and resolved-in-root-but-unreadable) so
/// they cannot be told apart — see `prepare_claim`'s doc comment.
const CONFINEMENT_REFUSAL: &str = "does not resolve inside the packet's directory";

/// The id namespace `resolve_artifact` mints into and nothing else may enter:
/// a claim naming an id here would let an actor holding only evidence-attach
/// permanently squat a content hash before any artifact for it exists, so
/// every later `--artifact` of those exact bytes finds a non-artifact "claim"
/// already sitting at that id and refuses with no repair path (retiring the
/// squatter does not help; `resolve_artifact` checks type, not lifecycle).
const ARTIFACT_ID_PREFIX: &str = "artifact:sha256-";

/// The artifact/claim boundary, computed once.
///
/// `evidence attach` is the only place that opens a named file and hashes it,
/// so it is the only place that may mint the `custom:artifact` cell whose id
/// names that hash (`src/native_model.rs::require_artifact_cell_entered_via_attach`
/// refuses one minted anywhere else). Two attachments naming the same bytes —
/// in the same command or across separate ones — must land on the one cell
/// already recorded for that hash, so a second citation adds only the
/// `derives_from` relation.
///
/// `artifact_path` must already be the canonical path `prepare_claim`
/// resolved and read the bytes from — this function records it verbatim as
/// `metadata.artifact_uri`, so a caller passing anything else would make that
/// field name a path the tool did not actually read.
fn resolve_artifact(
    case_space: &CaseSpace,
    staged_artifact_ids: &mut BTreeSet<Id>,
    claim_cell: &CaseCell,
    artifact_path: &Path,
    artifact_bytes: &[u8],
    relation_index: usize,
) -> Result<(Option<CaseCell>, CaseRelation), NativeCliError> {
    let content_hash = crate::native_hash::sha256_hex(artifact_bytes);
    let artifact_id = Id::new(format!("{ARTIFACT_ID_PREFIX}{content_hash}"))?;
    let already_recorded = match case_space
        .case_cells
        .iter()
        .find(|candidate| candidate.id == artifact_id)
    {
        Some(existing) => {
            let existing_hash = existing
                .metadata
                .get("content_hash")
                .and_then(Value::as_str);
            if !is_artifact_cell(existing) || existing_hash != Some(content_hash.as_str()) {
                return Err(NativeCliError::invalid(format!(
                    "artifact id {artifact_id} already exists as a cell that is not a matching \
                     custom:artifact"
                )));
            }
            true
        }
        None => staged_artifact_ids.contains(&artifact_id),
    };
    let artifact_cell = if already_recorded {
        None
    } else {
        staged_artifact_ids.insert(artifact_id.clone());
        let title = artifact_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| artifact_path.display().to_string());
        Some(CaseCell {
            id: artifact_id.clone(),
            cell_type: CaseCellType::Custom(ARTIFACT_CELL_TYPE.to_owned()),
            space_id: case_space.space_id.clone(),
            title,
            summary: None,
            // `resolved`: an artifact is a completed observation, not open
            // work, so it stays off the frontier and out of readiness
            // subjects the moment it is recorded.
            lifecycle: CaseCellLifecycle::Resolved,
            source_ids: claim_cell.source_ids.clone(),
            structure_ids: Vec::new(),
            provenance: provenance(
                SourceKind::Custom("tool_captured_artifact".to_owned()),
                ReviewStatus::Unreviewed,
            ),
            metadata: Map::from_iter([
                ("content_hash".to_owned(), json!(content_hash)),
                (
                    "artifact_uri".to_owned(),
                    json!(artifact_path.display().to_string()),
                ),
            ]),
        })
    };
    let relation = CaseRelation {
        id: Id::new(format!(
            "relation:derives-from:{}:{relation_index}",
            path_segment(&claim_cell.id)
        ))?,
        relation_type: CaseRelationType::DerivesFrom,
        relation_strength: RelationStrength::Diagnostic,
        from_id: claim_cell.id.clone(),
        to_id: artifact_id,
        evidence_ids: vec![claim_cell.id.clone()],
        source_ids: claim_cell.source_ids.clone(),
        provenance: claim_cell.provenance.clone(),
        metadata: Map::new(),
    };
    Ok((artifact_cell, relation))
}

fn claim_attachment_id(
    existing_ids: &BTreeSet<Id>,
    claimed_ids: &mut BTreeMap<Id, PathBuf>,
    id: &Id,
    input: &Path,
) -> Result<(), NativeCliError> {
    if existing_ids.contains(id) {
        return Err(input_refusal(input, format!("id {id} already exists")));
    }
    if let Some(first_input) = claimed_ids.insert(id.clone(), input.to_path_buf()) {
        return Err(input_refusal(
            input,
            format!(
                "added id {id} duplicates an id from input {}",
                first_input.display()
            ),
        ));
    }
    Ok(())
}

fn input_refusal(input: &Path, error: impl std::fmt::Display) -> NativeCliError {
    NativeCliError::invalid(format!(
        "evidence attach input {} was refused: {error}",
        input.display()
    ))
}

pub(in crate::native_cli) fn cell_transition(
    store: &Path,
    case_space_id: &Id,
    base_revision_id: &Id,
    cell_id: &Id,
    lifecycle: &str,
    reason: Option<&str>,
    gate_options: &NativeMutationGateOptions,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, base_revision_id)?;
    let target_lifecycle = parse_lifecycle(lifecycle)?;
    let operation_gate =
        validated_mutation_gate(&replay.case_space, gate_options, "cell-transition")?;
    append_cell_transition_morphism(
        &store_api,
        &replay.case_space,
        cell_id,
        target_lifecycle,
        reason,
        operation_gate,
        "casegraphen cell transition",
    )
}

/// The one place that turns a target cell and lifecycle into the Update
/// morphism that transitions it, and appends it. `cell transition` reaches
/// this directly; `packet resume` reaches it after verifying the actor seam
/// held — same gate operation, same morphism shape, same append.
pub(super) fn append_cell_transition_morphism(
    store_api: &NativeCaseStore,
    case_space: &CaseSpace,
    cell_id: &Id,
    target_lifecycle: CaseCellLifecycle,
    reason: Option<&str>,
    operation_gate: NativeOperationGate,
    command: &str,
) -> Result<Value, NativeCliError> {
    let mut morphism = cell_transition_morphism(case_space, cell_id, target_lifecycle, reason)?;
    morphism.metadata.insert(
        "operation_gate".to_owned(),
        serde_json::to_value(&operation_gate)?,
    );
    append_validated_morphism(
        store_api,
        case_space,
        morphism,
        Some(operation_gate.actor_id),
        command,
    )
}

fn cell_transition_morphism(
    case_space: &CaseSpace,
    cell_id: &Id,
    target_lifecycle: CaseCellLifecycle,
    reason: Option<&str>,
) -> Result<CaseMorphism, NativeCliError> {
    let mut updated_cell = case_space
        .case_cells
        .iter()
        .find(|cell| cell.id == *cell_id)
        .cloned()
        .ok_or_else(|| NativeCliError::invalid(format!("unknown cell id {cell_id}")))?;
    let source_lifecycle = updated_cell.lifecycle;
    updated_cell.lifecycle = target_lifecycle;

    let sequence = case_space.morphism_log.len() + 1;
    let mut metadata = Map::new();
    metadata.insert(
        "payload".to_owned(),
        serde_json::to_value(MorphismPayload {
            updated_cells: vec![updated_cell.clone()],
            ..MorphismPayload::default()
        })?,
    );
    metadata.insert(
        "transition".to_owned(),
        json!({
            "from": source_lifecycle,
            "to": target_lifecycle,
            "reason": reason,
        }),
    );
    Ok(CaseMorphism {
        morphism_id: generated_operation_id("morphism:cell-transition", cell_id, sequence)?,
        morphism_type: CaseMorphismType::Update,
        source_revision_id: Some(case_space.revision.revision_id.clone()),
        target_revision_id: generated_operation_id("revision:cell-transition", cell_id, sequence)?,
        added_ids: Vec::new(),
        updated_ids: vec![cell_id.clone()],
        retired_ids: Vec::new(),
        preserved_ids: Vec::new(),
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: Vec::new(),
        source_ids: updated_cell.source_ids.clone(),
        metadata,
    })
}

pub(super) fn evidence_cell_from_bytes(bytes: &[u8]) -> Result<CaseCell, NativeCliError> {
    let mut cell: CaseCell = super::io::parse_strict(serde_json::from_slice(bytes)?)?;
    if cell.cell_type != CaseCellType::Evidence {
        return Err(NativeCliError::invalid(format!(
            "evidence attach input cell {} has cell_type {}; expected evidence",
            cell.id, cell.cell_type
        )));
    }
    if cell.provenance.review_status == ReviewStatus::Accepted {
        return Err(NativeCliError::invalid(format!(
            "evidence attach input cell {} cannot claim accepted provenance; use review accept to promote evidence",
            cell.id
        )));
    }
    // The artifact namespace is minted content, not a claim id a caller
    // chooses: an actor holding only evidence-attach could otherwise squat a
    // content hash before any artifact for it exists, and every later
    // `--artifact` of those exact bytes would find a non-artifact cell
    // already there with no way to repair it.
    if cell.id.as_str().starts_with(ARTIFACT_ID_PREFIX) {
        return Err(NativeCliError::invalid(format!(
            "evidence attach input cell {} claims the artifact namespace ({ARTIFACT_ID_PREFIX}...); \
             that id space is minted only by attaching an artifact, never chosen by a claim",
            cell.id
        )));
    }
    // Attached evidence is untrusted by construction: the caller does not get
    // to name its boundary, and `inferred` is the spelling the shared trust
    // rule reads as "needs an accepted review".
    cell.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String(EvidenceTrustBoundary::Inferred.metadata_value().to_owned()),
    );
    cell.metadata.insert(
        "content_hash".to_owned(),
        Value::String(crate::native_hash::sha256_hex(bytes)),
    );
    Ok(cell)
}

fn parse_lifecycle(value: &str) -> Result<CaseCellLifecycle, NativeCliError> {
    serde_json::from_value(Value::String(value.to_owned()))
        .map_err(|error| NativeCliError::invalid(format!("invalid lifecycle {value:?}: {error}")))
}

fn review_target_kind(
    case_space: &CaseSpace,
    target_id: &Id,
) -> Result<NativeReviewTargetKind, NativeCliError> {
    if let Some(cell) = case_space
        .case_cells
        .iter()
        .find(|cell| cell.id == *target_id)
    {
        // An artifact is an observation, not a claim, so it falling through to
        // `Waiver` below would have let `review accept` promote it as though
        // it were a reviewable target. What is reviewable is the evidence
        // cell that cites the artifact with a `derives_from` relation.
        if is_artifact_cell(cell) {
            return Err(NativeCliError::invalid(format!(
                "cannot review artifact cell {target_id}: an artifact is an observation, not a \
                 claim; review the evidence cell that derives from it instead"
            )));
        }
        return Ok(match cell.cell_type {
            CaseCellType::Completion => NativeReviewTargetKind::Completion,
            CaseCellType::Evidence => NativeReviewTargetKind::Evidence,
            _ => NativeReviewTargetKind::Waiver,
        });
    }
    if case_space
        .case_relations
        .iter()
        .any(|relation| relation.id == *target_id)
    {
        return Ok(NativeReviewTargetKind::Waiver);
    }
    if case_space
        .morphism_log
        .iter()
        .any(|entry| entry.morphism_id == *target_id)
    {
        return Ok(NativeReviewTargetKind::Morphism);
    }
    // An obstruction id names neither a cell, a relation, nor a morphism, so
    // it falls through the three checks above — it is checked last, and
    // against a derived evaluation rather than stored state, because that is
    // the only place obstruction ids exist. `ResidualRisk` is the target kind
    // `require_review_request` (`native_review.rs:871`) already validates an
    // obstruction id against, via `require_obstruction_target`; this is the
    // dispatcher route that was missing to reach it (#158).
    if evaluate_native_case(case_space)?
        .obstructions
        .iter()
        .any(|obstruction| obstruction.id == *target_id)
    {
        return Ok(NativeReviewTargetKind::ResidualRisk);
    }
    Err(NativeCliError::invalid(format!(
        "unknown review target {target_id}"
    )))
}

fn generated_revision_id(
    case_space: &CaseSpace,
    operation: &str,
    subject_id: &Id,
) -> Result<Id, NativeCliError> {
    generated_operation_id(
        &format!("revision:{operation}"),
        subject_id,
        case_space.morphism_log.len() + 1,
    )
}

fn generated_operation_id(
    prefix: &str,
    subject_id: &Id,
    sequence: usize,
) -> Result<Id, NativeCliError> {
    Ok(Id::new(format!(
        "{prefix}:{}:{sequence}",
        path_segment(subject_id)
    ))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_model::Revision;

    const EVIDENCE_CELL: &[u8] = br#"{
        "id": "evidence:unit-test",
        "cell_type": "evidence",
        "space_id": "space:unit-test",
        "title": "Unit test evidence",
        "lifecycle": "active",
        "source_ids": ["source:unit-test"],
        "structure_ids": [],
        "provenance": {
            "source": {"kind": "document", "title": "Unit test"},
            "confidence": 1.0,
            "review_status": "unreviewed"
        },
        "metadata": {}
    }"#;

    #[test]
    fn evidence_cell_validation_adds_bare_sha256_content_hash() {
        let cell = evidence_cell_from_bytes(EVIDENCE_CELL).expect("valid evidence cell");
        let hash = cell.metadata["content_hash"]
            .as_str()
            .expect("content hash");

        assert_eq!(hash, crate::native_hash::sha256_hex(EVIDENCE_CELL));
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(cell.provenance.review_status, ReviewStatus::Unreviewed);
    }

    #[test]
    fn evidence_cell_validation_overwrites_caller_content_hash() {
        let bytes = String::from_utf8(EVIDENCE_CELL.to_vec())
            .expect("UTF-8 fixture")
            .replace(
                "\"metadata\": {}",
                "\"metadata\": {\"content_hash\":\"bogus\"}",
            );
        let cell = evidence_cell_from_bytes(bytes.as_bytes()).expect("valid evidence cell");

        assert_eq!(
            cell.metadata["content_hash"],
            json!(crate::native_hash::sha256_hex(bytes.as_bytes()))
        );
    }

    #[test]
    fn evidence_cell_validation_overwrites_caller_claimed_boundary_and_rejects_provenance() {
        let accepted_boundary = String::from_utf8(EVIDENCE_CELL.to_vec())
            .expect("UTF-8 fixture")
            .replace(
                "\"metadata\": {}",
                "\"metadata\": {\"evidence_boundary\":\"accepted_evidence\"}",
            );
        let cell = evidence_cell_from_bytes(accepted_boundary.as_bytes())
            .expect("caller boundary is overwritten");
        // The stored spelling must be one the shared trust rule actually reads
        // as untrusted, not merely an unrecognized string that happens to fall
        // through to `Inferred`.
        assert_eq!(
            cell.metadata["evidence_boundary"],
            json!(EvidenceTrustBoundary::Inferred.metadata_value())
        );

        let accepted_review = String::from_utf8(EVIDENCE_CELL.to_vec())
            .expect("UTF-8 fixture")
            .replace(
                "\"review_status\": \"unreviewed\"",
                "\"review_status\": \"accepted\"",
            );
        let review_error = evidence_cell_from_bytes(accepted_review.as_bytes())
            .expect_err("accepted provenance must require review");
        assert!(review_error.to_string().contains("review accept"));
    }

    #[test]
    fn evidence_cell_validation_rejects_non_evidence_cells() {
        let bytes = String::from_utf8(EVIDENCE_CELL.to_vec())
            .expect("UTF-8 fixture")
            .replace("\"evidence\"", "\"work\"");
        let error = evidence_cell_from_bytes(bytes.as_bytes()).expect_err("reject work cell");

        assert!(error.to_string().contains("expected evidence"));
    }

    #[test]
    fn evidence_cell_validation_refuses_a_claim_id_in_the_artifact_namespace() {
        // Otherwise an actor holding only evidence-attach could squat a
        // content hash under `artifact:sha256-...` before any artifact for
        // it exists, with no repair path once squatted.
        let bytes = String::from_utf8(EVIDENCE_CELL.to_vec())
            .expect("UTF-8 fixture")
            .replace("evidence:unit-test", "artifact:sha256-deadbeef");
        let error =
            evidence_cell_from_bytes(bytes.as_bytes()).expect_err("reject artifact-namespace id");

        assert!(error.to_string().contains("claims the artifact namespace"));
    }

    #[test]
    fn lifecycle_parser_uses_serde_names_and_lists_valid_values() {
        assert_eq!(
            parse_lifecycle("resolved").expect("resolved lifecycle"),
            CaseCellLifecycle::Resolved
        );
        let error = parse_lifecycle("Resolved").expect_err("serde names are lowercase");
        let message = error.to_string();
        assert!(message.contains("proposed"));
        assert!(message.contains("superseded"));
    }

    fn minimal_case_space(cells: Vec<CaseCell>) -> CaseSpace {
        CaseSpace {
            schema: crate::native_model::NATIVE_CASE_SPACE_SCHEMA.to_owned(),
            schema_version: crate::native_model::NATIVE_CASE_SPACE_SCHEMA_VERSION,
            case_space_id: Id::new("case_space:unit-test").expect("id"),
            space_id: Id::new("space:unit-test").expect("id"),
            case_cells: cells,
            case_relations: Vec::new(),
            morphism_log: Vec::new(),
            projections: Vec::new(),
            revision: Revision {
                revision_id: Id::new("revision:unit-test").expect("id"),
                case_space_id: Id::new("case_space:unit-test").expect("id"),
                applied_entry_ids: Vec::new(),
                applied_morphism_ids: Vec::new(),
                checksum: String::new(),
                parent_revision_id: None,
                created_at: "unix:0".to_owned(),
                source_ids: Vec::new(),
                metadata: Map::new(),
            },
            close_policy_id: None,
            metadata: Map::new(),
        }
    }

    #[test]
    fn review_target_kind_refuses_an_artifact_cell() {
        let artifact = CaseCell {
            id: Id::new("artifact:sha256-unit-test").expect("id"),
            cell_type: CaseCellType::Custom(ARTIFACT_CELL_TYPE.to_owned()),
            space_id: Id::new("space:unit-test").expect("id"),
            title: "Unit test artifact".to_owned(),
            summary: None,
            lifecycle: CaseCellLifecycle::Resolved,
            source_ids: Vec::new(),
            structure_ids: Vec::new(),
            provenance: provenance(
                SourceKind::Custom("tool_captured_artifact".to_owned()),
                ReviewStatus::Unreviewed,
            ),
            metadata: Map::new(),
        };
        let target_id = artifact.id.clone();
        let space = minimal_case_space(vec![artifact]);

        let error = review_target_kind(&space, &target_id).expect_err("artifact refuses review");
        assert!(
            error.to_string().contains("an observation, not a claim"),
            "{error}"
        );
    }

    #[test]
    fn resolve_artifact_dedupes_when_the_same_hash_is_already_recorded() {
        let claim = evidence_cell_from_bytes(EVIDENCE_CELL).expect("claim cell");
        let mut staged = BTreeSet::new();
        let bytes = b"artifact bytes shared by two citations";
        let space = minimal_case_space(Vec::new());

        let (first_cell, first_relation) = resolve_artifact(
            &space,
            &mut staged,
            &claim,
            Path::new("first.log"),
            bytes,
            1,
        )
        .expect("first citation resolves");
        let artifact_cell = first_cell.expect("the first citation mints the artifact");

        let mut space_with_artifact = space;
        space_with_artifact.case_cells.push(artifact_cell);
        let (second_cell, second_relation) = resolve_artifact(
            &space_with_artifact,
            &mut staged,
            &claim,
            Path::new("second.log"),
            bytes,
            2,
        )
        .expect("second citation resolves");

        assert!(
            second_cell.is_none(),
            "identical bytes must not mint a second artifact cell"
        );
        assert_eq!(second_relation.to_id, first_relation.to_id);
        assert_ne!(first_relation.id, second_relation.id);
    }

    #[test]
    fn resolve_artifact_refuses_a_hash_collision_with_a_non_artifact_cell() {
        let claim = evidence_cell_from_bytes(EVIDENCE_CELL).expect("claim cell");
        let bytes = b"bytes whose hash a non-artifact cell already claims";
        let content_hash = crate::native_hash::sha256_hex(bytes);
        let mut colliding = claim.clone();
        colliding.id = Id::new(format!("artifact:sha256-{content_hash}")).expect("id");
        let space = minimal_case_space(vec![colliding]);
        let mut staged = BTreeSet::new();

        let error = resolve_artifact(
            &space,
            &mut staged,
            &claim,
            Path::new("colliding.log"),
            bytes,
            1,
        )
        .expect_err("a non-artifact cell at the same content-addressed id must refuse");
        assert!(
            error
                .to_string()
                .contains("already exists as a cell that is not a matching"),
            "{error}"
        );
    }

    fn unique_scratch_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "casegraphen-mutations-test-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    // The containment predicate itself (`path_confined`, exercised here via
    // `artifact_confined_matches_which_file_a_generated_path_actually_reaches`
    // below) has its own unit test at `crate::path_confinement::tests` — it
    // is the single shared implementation now, not a copy owned by this
    // module.

    /// Extracts the message from a `NativeCliError::Invalid` refusal,
    /// panicking on any other variant or on `Ok`. Used instead of comparing
    /// `.to_string()` output so an assertion that two refusals are identical
    /// also proves both are the same *kind* of refusal, not merely two
    /// values that happen to render the same text.
    ///
    /// **This is the guard against reopening issue #21's existence oracle**,
    /// and every confined-artifact test in this module goes through it: the
    /// `Err(NativeCliError::Invalid(message))` **pattern** below only
    /// matches production code that actually constructed `Invalid`. If a
    /// future change carved one of the three confined failure modes —
    /// lexical rejection, canonicalization failure, resolved-but-outside —
    /// into its own variant, the real `prepare_claim` call for that mode
    /// would return that new variant instead, and this pattern would fail
    /// to match and panic via the `Err(other)` arm below — on the real code
    /// path, not on a copy of it. The `assert_eq!` on `error_code()` right
    /// after is *not* that guard: it rebuilds a fresh `Invalid` from the
    /// already-extracted message and checks that copy's own code, which is
    /// "invalid" unconditionally — a tautology kept here only as a
    /// documentation-level reminder of what code this variant carries, not
    /// as enforcement.
    fn expect_confined_refusal(
        result: Result<PreparedEvidenceAttachment, NativeCliError>,
    ) -> String {
        match result {
            Err(NativeCliError::Invalid(message)) => {
                assert_eq!(
                    NativeCliError::Invalid(message.clone()).error_code(),
                    "invalid",
                    "a confined-artifact refusal must keep the single generic error_code"
                );
                message
            }
            Err(other) => panic!("expected NativeCliError::Invalid, got {other:?}"),
            Ok(_) => panic!("expected the confined artifact to be refused"),
        }
    }

    #[test]
    fn prepare_claim_confines_a_packet_artifact_but_leaves_evidence_attach_unconfined() {
        let base = unique_scratch_dir("confine");
        let packet_dir = base.join("packet");
        fs::create_dir_all(&packet_dir).expect("create packet dir");
        fs::write(packet_dir.join("inside.txt"), b"inside bytes").expect("write inside artifact");
        let outside_target = base.join("outside.txt");
        fs::write(&outside_target, b"outside bytes").expect("write outside artifact");
        let claim = evidence_cell_from_bytes(EVIDENCE_CELL).expect("claim cell");
        let space = minimal_case_space(Vec::new());
        let existing_ids: BTreeSet<Id> = BTreeSet::new();
        let canonical_packet_dir = fs::canonicalize(&packet_dir).expect("canonicalize packet dir");
        let prepare = |artifact: &str| {
            let mut state = ClaimPreparationState::new(&existing_ids);
            prepare_claim(
                &space,
                &mut state,
                Path::new("packet.json"),
                claim.clone(),
                &[],
                std::slice::from_ref(&PathBuf::from(artifact)),
                Some(&canonical_packet_dir),
            )
        };

        // Confined: a plain relative entry inside the packet directory is
        // accepted, and the recorded uri is the canonical path that was
        // actually read — `prepare_claim` does the one join, onto the same
        // root the containment check uses.
        let prepared = prepare("inside.txt").expect("an artifact inside the root is accepted");
        let artifact_cell = prepared
            .artifacts
            .first()
            .expect("one artifact cell minted");
        assert_eq!(
            artifact_cell.metadata["artifact_uri"],
            json!(canonical_packet_dir
                .join("inside.txt")
                .to_str()
                .expect("canonical artifact path"))
        );

        // Lexical rejection, stage 1: an absolute entry is refused before any
        // filesystem call, even though the path it names genuinely exists.
        // If this stage did not run, `confine_within.join(absolute)` would
        // just be `absolute` (`Path::join` discards the base for an absolute
        // addition) and canonicalizing it would succeed — that is issue #21
        // defect 2's re-opened oracle. The echoed label differs from the
        // other cases below (each preserves its own caller-supplied string),
        // so only the shared reason is compared here, not the whole message.
        let absolute_error = expect_confined_refusal(prepare(
            outside_target.to_str().expect("outside path is UTF-8"),
        ));
        assert!(
            absolute_error.ends_with(CONFINEMENT_REFUSAL),
            "{absolute_error}"
        );

        // Lexical rejection, stage 1: a relative entry containing `..` is
        // refused before any filesystem call too, even though it genuinely
        // resolves to an existing file.
        let dotdot_error = expect_confined_refusal(prepare("../outside.txt"));
        assert!(
            dotdot_error.ends_with(CONFINEMENT_REFUSAL),
            "{dotdot_error}"
        );

        // The existence-oracle proof: the identical entry, refused first
        // because it names nothing (stage 2, canonicalization fails), then
        // refused again — byte for byte the same message — once it is
        // backed by a real, existing symlink that escapes the root (stage 3,
        // containment fails). Lexical rejection cannot see this case (the
        // entry is a plain relative name), so this is exactly the pairing
        // issue #21 defect 2 needed: "does not exist" and "exists but
        // escapes" must be indistinguishable to the caller.
        let missing_error = expect_confined_refusal(prepare("escape-link"));
        assert!(
            missing_error.ends_with(CONFINEMENT_REFUSAL),
            "{missing_error}"
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside_target, packet_dir.join("escape-link"))
                .expect("create escaping symlink");
            let escaping_error = expect_confined_refusal(prepare("escape-link"));
            assert_eq!(
                missing_error, escaping_error,
                "a missing entry and the identical entry once backed by an escaping symlink \
                 must refuse identically"
            );
        }

        // A fourth confined failure mode: an entry that resolves in-root
        // and passes containment but cannot be read, because it names a
        // directory rather than a file. Before the fix, this reported the
        // raw io error ("Is a directory"), splitting "exists as a
        // directory" from "does not exist" (`missing_error` above) for any
        // relative name inside the packet's own directory — a smaller
        // version of the same oracle class, still worth closing. The
        // message must match the other three modes exactly, not merely end
        // with the shared suffix, since all four are meant to be one
        // refusal.
        fs::create_dir_all(packet_dir.join("inside-dir")).expect("create in-root subdirectory");
        let directory_error = expect_confined_refusal(prepare("inside-dir"));
        assert!(
            directory_error.ends_with(CONFINEMENT_REFUSAL),
            "{directory_error}"
        );

        // Unconfined: the identical escaping path is accepted with no
        // confinement root — `evidence attach --artifact` is operator-typed,
        // same as every other input flag, and gets no lexical check.
        let mut unconfined_state = ClaimPreparationState::new(&existing_ids);
        let unconfined = prepare_claim(
            &space,
            &mut unconfined_state,
            Path::new("attach-input.json"),
            claim,
            &[],
            std::slice::from_ref(&outside_target),
            None,
        )
        .expect("evidence attach stays unconfined");
        assert_eq!(unconfined.artifacts.len(), 1);

        fs::remove_dir_all(&base).ok();
    }

    /// An absolute artifact string that climbs from `probe_dir` up to `/`
    /// (one `..` per named component — `..` at `/` is a no-op, so this must
    /// reach the real root) and back down through `root`'s own absolute path
    /// to `filename`. `fs::canonicalize` validates every component along the
    /// way, so this string canonicalizes successfully, to a real file inside
    /// `root`, if and only if `probe_dir` exists — that is the existence
    /// oracle issue #21 defect 2 reopened, with no symlink required.
    fn climb_to_root_and_descend_into(probe_dir: &Path, root: &Path, filename: &str) -> String {
        // A generous, fixed climb count rather than `probe_dir`'s own
        // component count: `..` resolution is physical, following symlinks
        // as it goes, so a symlinked ancestor (macOS's `/var` ->
        // `/private/var`, which is exactly why `/etc/../..` does not reach
        // `/` in one hop) makes the real resolved depth deeper than the raw
        // string's component count. `..` at `/` is a no-op, so climbing
        // further than needed is always safe; climbing short of it is not —
        // it undercounted here once already and produced a bogus path that
        // failed to canonicalize regardless of whether `probe_dir` existed,
        // silently defeating the property this test exists to check.
        const GENEROUS_CLIMB_COUNT: usize = 64;
        let climb = "../".repeat(GENEROUS_CLIMB_COUNT);
        let root_suffix = root
            .strip_prefix("/")
            .expect("root must be absolute")
            .to_str()
            .expect("root is UTF-8");
        format!(
            "{}/{climb}{root_suffix}/{filename}",
            probe_dir.to_str().expect("probe dir is UTF-8")
        )
    }

    #[test]
    fn prepare_claim_lexical_rejection_closes_the_climb_and_return_existence_oracle() {
        let base = unique_scratch_dir("climb-oracle");
        let packet_dir = base.join("packet");
        fs::create_dir_all(&packet_dir).expect("create packet dir");
        fs::write(packet_dir.join("inside.txt"), b"inside bytes").expect("write inside artifact");
        let claim = evidence_cell_from_bytes(EVIDENCE_CELL).expect("claim cell");
        let space = minimal_case_space(Vec::new());
        let existing_ids: BTreeSet<Id> = BTreeSet::new();
        let canonical_packet_dir = fs::canonicalize(&packet_dir).expect("canonicalize packet dir");
        let prepare = |artifact: String| {
            let mut state = ClaimPreparationState::new(&existing_ids);
            prepare_claim(
                &space,
                &mut state,
                Path::new("packet.json"),
                claim.clone(),
                &[],
                std::slice::from_ref(&PathBuf::from(artifact)),
                Some(&canonical_packet_dir),
            )
        };

        // `base` genuinely exists; a sibling under it does not. Both are
        // used only as the climbed-through absolute prefix — the string
        // always resolves back down into `canonical_packet_dir/inside.txt`,
        // a real file that legitimately belongs in the root.
        let existing_probe = base.clone();
        let missing_probe = base.join("zzz-does-not-exist-probe");
        assert!(
            !missing_probe.exists(),
            "probe must not exist for this test to mean anything"
        );

        let existing_probe_artifact =
            climb_to_root_and_descend_into(&existing_probe, &canonical_packet_dir, "inside.txt");
        let missing_probe_artifact =
            climb_to_root_and_descend_into(&missing_probe, &canonical_packet_dir, "inside.txt");

        // Prove the crafted string is a genuine working exploit shape before
        // proving it is refused — otherwise a mistake in the climb (as
        // happened once while writing this test: too few `..` left a bogus
        // path that failed to canonicalize regardless of `probe_dir`,
        // silently making the test pass for the wrong reason) would make
        // this test meaningless. Bypassing `prepare_claim` entirely and
        // canonicalizing the raw string directly: it resolves to the real
        // in-root file exactly when the climbed-through directory exists.
        assert_eq!(
            fs::canonicalize(&existing_probe_artifact).expect("the exploit string must resolve"),
            canonical_packet_dir.join("inside.txt"),
            "the crafted string must genuinely reach the real in-root file when unconfined"
        );
        assert!(
            fs::canonicalize(&missing_probe_artifact).is_err(),
            "the crafted string must genuinely fail to resolve when the probed directory is \
             absent — that gap between success and failure is the oracle"
        );

        // Yet `prepare_claim`, confined, refuses both identically: lexical
        // rejection fires on the absolute string before any filesystem call,
        // so whether `base` or its nonexistent sibling exists is never
        // observed.
        let existing_probe_error = expect_confined_refusal(prepare(existing_probe_artifact));
        let missing_probe_error = expect_confined_refusal(prepare(missing_probe_artifact));
        assert!(
            existing_probe_error.ends_with(CONFINEMENT_REFUSAL),
            "{existing_probe_error}"
        );
        assert!(
            missing_probe_error.ends_with(CONFINEMENT_REFUSAL),
            "{missing_probe_error}"
        );

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn artifact_confined_matches_which_file_a_generated_path_actually_reaches() {
        // Real files and a real symlink, not a string model of one: the
        // property is that the confinement check agrees with the OS's own
        // resolution of `..`, `.`, and an escaping symlink, not with a
        // second implementation of path-escape reasoning.
        let base = unique_scratch_dir("confinement-property");
        let root = base.join("packet");
        fs::create_dir_all(root.join("sub")).expect("create packet tree");
        fs::create_dir_all(base.join("outside")).expect("create outside dir");
        fs::write(root.join("inside.txt"), b"INSIDE").expect("write inside file");
        fs::write(root.join("sub").join("inside2.txt"), b"INSIDE")
            .expect("write nested inside file");
        fs::write(base.join("outside").join("secret.txt"), b"OUTSIDE").expect("write outside file");
        #[cfg(unix)]
        std::os::unix::fs::symlink(base.join("outside"), root.join("escape"))
            .expect("create escaping symlink");

        let canonical_root = fs::canonicalize(&root).expect("canonicalize root");
        let canonical_base = fs::canonicalize(&base).expect("canonicalize base");
        // Owned so the absolute anchors can live in the segment alphabet
        // below: `PathBuf::push` with an absolute component discards
        // whatever was pushed before it (same as `Path::join`), so including
        // these lets a generated sequence begin outside `root` entirely —
        // the shape `prepare_claim`'s lexical rejection exists to refuse —
        // and then climb further above it or descend back down through
        // `packet`, not only ever descend from `root` itself.
        let base_absolute = canonical_base.to_str().expect("base is UTF-8").to_owned();
        let root_absolute = canonical_root.to_str().expect("root is UTF-8").to_owned();
        let segments: Vec<&str> = vec![
            "..",
            ".",
            "inside.txt",
            "sub",
            "escape",
            "outside",
            "secret.txt",
            "missing.txt",
            "packet",
            base_absolute.as_str(),
            root_absolute.as_str(),
        ];

        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let len: usize = u.int_in_range(1..=6)?;
                let mut relative = PathBuf::new();
                for _ in 0..len {
                    relative.push(*u.choose(&segments)?);
                }
                let candidate = root.join(&relative);

                // Only a path that actually reaches one of the two marker
                // files says anything about confinement; a directory, a
                // dangling `missing.txt`, or a symlink loop reads as an
                // ordinary `Err` here and is not this property's concern.
                let Ok(content) = fs::read(&candidate) else {
                    return Ok(());
                };
                let ground_truth_inside = match content.as_slice() {
                    b"INSIDE" => true,
                    b"OUTSIDE" => false,
                    _ => return Ok(()),
                };
                let canonical_candidate =
                    fs::canonicalize(&candidate).expect("a path that read must canonicalize");

                assert_eq!(
                    path_confined(&canonical_candidate, &canonical_root),
                    ground_truth_inside,
                    "confinement check disagreed with which file {relative:?} actually reaches"
                );
                Ok(())
            },
        );

        fs::remove_dir_all(&base).ok();
    }
}

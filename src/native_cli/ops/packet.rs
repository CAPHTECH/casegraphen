//! `packet apply` and `packet resume`: the actor-seam layer over evidence
//! attach and cell transition (ADR 0015).
//!
//! A packet never performs a review itself — `packet apply` always pauses
//! after the attach, and `packet resume` refuses unless an independent review
//! already landed in the log. Both commands are thin: they read one strict
//! JSON input and then call the exact same shared functions `evidence attach`
//! and `cell transition` call, so a rule enforced for the plain commands is
//! enforced here too, not re-decided.

use super::{
    existing_case_space_ids, prepare_claim, require_current_revision, validated_mutation_gate,
    ClaimPreparationState, NativeMutationGateOptions,
};
use crate::{
    native_eval::latest_evidence_review_status,
    native_model::{CaseCell, CaseCellLifecycle, CaseCellType, CaseMorphismType},
    native_store::NativeCaseStore,
};
use higher_graphen_core::{Id, ReviewStatus};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

use super::super::NativeCliError;
use super::mutations::{append_cell_transition_morphism, append_evidence_attach_morphism};

pub const EVIDENCE_PACKET_SCHEMA: &str = "highergraphen.case.evidence_packet.v1";
const EVIDENCE_PACKET_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidencePacket {
    schema: String,
    schema_version: u32,
    case_space_id: Id,
    target: EvidencePacketTarget,
    claim: CaseCell,
    // Required keys, but the array itself may be empty — see the schema.
    artifacts: Vec<String>,
    satisfies: Vec<Id>,
    completion: EvidencePacketCompletion,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidencePacketTarget {
    cell_id: Id,
    transition_to: CaseCellLifecycle,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidencePacketCompletion {
    reason: String,
}

fn read_evidence_packet(path: &Path, case_space_id: &Id) -> Result<EvidencePacket, NativeCliError> {
    let packet: EvidencePacket = super::io::parse_strict(super::io::read_json(path)?)?;
    if packet.schema != EVIDENCE_PACKET_SCHEMA {
        return Err(NativeCliError::invalid(format!(
            "{}: unsupported evidence packet schema {:?}; expected {EVIDENCE_PACKET_SCHEMA:?}",
            path.display(),
            packet.schema
        )));
    }
    if packet.schema_version != EVIDENCE_PACKET_SCHEMA_VERSION {
        return Err(NativeCliError::invalid(format!(
            "{}: unsupported evidence packet schema version {}; expected {EVIDENCE_PACKET_SCHEMA_VERSION}",
            path.display(),
            packet.schema_version
        )));
    }
    // Refused before any mutation: a packet is portable text, and nothing
    // stops one written for a different case space from being pointed at the
    // wrong store by mistake.
    if packet.case_space_id != *case_space_id {
        return Err(NativeCliError::invalid(format!(
            "packet case_space_id {} does not match --case-space-id {case_space_id}",
            packet.case_space_id
        )));
    }
    Ok(packet)
}

pub(in crate::native_cli) fn packet_apply(
    store: &Path,
    case_space_id: &Id,
    base_revision_id: &Id,
    packet_path: &Path,
    gate_options: &NativeMutationGateOptions,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, base_revision_id)?;
    let packet = read_evidence_packet(packet_path, case_space_id)?;
    // Gate operation is `evidence-attach`, the attach's own operation string:
    // apply performs exactly the attach it delegates to, no new vocabulary.
    let operation_gate =
        validated_mutation_gate(&replay.case_space, gate_options, "evidence-attach")?;

    let packet_directory = packet_path.parent().unwrap_or_else(|| Path::new("."));
    let artifact_paths = packet
        .artifacts
        .iter()
        .map(|artifact| packet_directory.join(artifact))
        .collect::<Vec<_>>();
    // The claim goes through the identical forced-inferred/hash/refusal
    // pipeline `--input` does: `evidence_cell_from_bytes` reads the claim's
    // own canonical serialization, not the caller's, exactly as it reads the
    // bytes of an attached file rather than trusting anything the file claims.
    let claim_bytes = serde_json::to_vec(&packet.claim)?;
    let claim_cell = super::mutations::evidence_cell_from_bytes(&claim_bytes)
        .map_err(|error| packet_refusal(packet_path, error))?;
    let existing_ids = existing_case_space_ids(&replay.case_space);
    let mut state = ClaimPreparationState::new(&existing_ids);
    let prepared = prepare_claim(
        &replay.case_space,
        &mut state,
        packet_path,
        claim_cell,
        &packet.satisfies,
        &artifact_paths,
    )
    .map_err(|error| packet_refusal(packet_path, error))?;
    let claim_cell_id = prepared.claim_cell_id().clone();
    let artifact_cell_ids = prepared.artifact_cell_ids().cloned().collect::<Vec<_>>();

    let mut result = append_evidence_attach_morphism(
        &store_api,
        &replay.case_space,
        vec![prepared],
        operation_gate,
        "casegraphen packet apply",
    )?;
    let completed_through = result["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("append result carries record.current_revision_id")
        .to_owned();
    if let Some(object) = result["result"].as_object_mut() {
        object.insert("status".to_owned(), json!("paused_for_review"));
        object.insert("claim_cell_id".to_owned(), json!(claim_cell_id));
        object.insert("artifact_cell_ids".to_owned(), json!(artifact_cell_ids));
        object.insert("completed_through".to_owned(), json!(completed_through));
        // Structured, not assembled shell text: a packet author controls
        // `claim.id`, and interpolating it into a command string an operator
        // is told to paste would let one `claim.id` value inject extra flags
        // — including `--actor-id`/`--capability-id` on the very `review
        // accept` this pause exists to keep independent. Each field here is a
        // named value, not a shell token; a caller assembles argv itself.
        object.insert(
            "next_operations".to_owned(),
            json!([
                {
                    "command": "review accept",
                    "store": store.display().to_string(),
                    "case_space_id": case_space_id,
                    "target_id": claim_cell_id,
                    "evidence_id": claim_cell_id,
                    "base_revision_id": completed_through,
                    "note": "must run under a different actor's gate holding the review \
                             operation, not this apply's actor or capability",
                },
                {
                    "command": "packet resume",
                    "store": store.display().to_string(),
                    "case_space_id": case_space_id,
                    "packet": packet_path.display().to_string(),
                    "completed_through": completed_through,
                    "note": "base_revision_id is whatever review accept's response reports as \
                             current_revision_id; run after that review is accepted",
                },
            ]),
        );
    }
    Ok(result)
}

pub(in crate::native_cli) fn packet_resume(
    store: &Path,
    case_space_id: &Id,
    base_revision_id: &Id,
    packet_path: &Path,
    completed_through: &Id,
    gate_options: &NativeMutationGateOptions,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, base_revision_id)?;
    // Authorize before touching the packet or the store's contents, the same
    // ordering `evidence attach` documents and `packet apply` already follows:
    // reading them first would let an actor holding no capability distinguish
    // a missing packet from a malformed one, or a known claim id from an
    // unknown one, through the refusal text alone.
    let operation_gate =
        validated_mutation_gate(&replay.case_space, gate_options, "cell-transition")?;
    let packet = read_evidence_packet(packet_path, case_space_id)?;

    // `--completed-through` is an assertion (ADR 0008/0014): the operator
    // names the revision `packet apply` produced, and the tool checks it
    // against history rather than inferring liveness from the graph's shape.
    // Absence here is a tool failure — a stale store, a rollback, or the
    // wrong space — not a rebase the tool should paper over.
    if !replay
        .history
        .iter()
        .any(|entry| entry.target_revision_id == *completed_through)
    {
        return Err(NativeCliError::invalid(format!(
            "packet resume refused: completed-through revision {completed_through} is not in \
             this case space's history (stale store, rollback, or wrong space)"
        )));
    }

    let claim_cell = replay
        .case_space
        .case_cells
        .iter()
        .find(|cell| cell.id == packet.claim.id)
        .ok_or_else(|| {
            NativeCliError::invalid(format!(
                "packet resume refused: claim cell {} does not exist in this case space",
                packet.claim.id
            ))
        })?;
    if claim_cell.cell_type != CaseCellType::Evidence {
        return Err(NativeCliError::invalid(format!(
            "packet resume refused: claim {} is not an evidence cell; a packet claim must be \
             cell_type evidence",
            claim_cell.id
        )));
    }
    // The claim must be the evidence *this packet's own apply* attached, not
    // any evidence cell of that id sitting elsewhere in the case space — else
    // a packet whose `claim.id` named a different, already-accepted attach's
    // claim could ride that accept to authorize a transition it was never
    // reviewed for. `--completed-through` names the one revision `packet
    // apply` reported producing, so the entry at exactly that revision must
    // be the `EvidenceAttach` morphism that added this claim — an id
    // attached earlier, at any other revision, by any other attach, is a
    // different claim wearing this one's id.
    let completed_through_entry = replay
        .history
        .iter()
        .find(|entry| entry.target_revision_id == *completed_through)
        .expect("completed_through was already verified present in history");
    let claim_was_attached_by_this_packet = completed_through_entry.morphism.morphism_type
        == CaseMorphismType::EvidenceAttach
        && completed_through_entry
            .morphism
            .added_ids
            .contains(&packet.claim.id);
    if !claim_was_attached_by_this_packet {
        return Err(NativeCliError::invalid(format!(
            "packet resume refused: claim {} was not added by the EvidenceAttach morphism at \
             completed-through revision {completed_through}; the claim must be the evidence \
             this packet's own apply attached",
            claim_cell.id
        )));
    }
    // The log-derived status only, with NO fallback to the cell's own stored
    // `provenance.review_status`: on a path that authorizes a durable
    // mutation, "no review in the log" must never read as accepted. See
    // `latest_evidence_review_status`'s doc comment for why this must not be
    // `effective_evidence_review_status`, which the findings section uses and
    // which legitimately falls back to the stored status.
    let log_review_status =
        latest_evidence_review_status(&replay.case_space, claim_cell.id.as_str());
    if log_review_status != Some(ReviewStatus::Accepted) {
        return Err(NativeCliError::invalid(format!(
            "packet resume refused: claim {} is not accepted by a canonical review in the log \
             (log-derived status: {:?}); run `casegraphen review accept --target-id {} ...` \
             under an independent actor's gate first",
            claim_cell.id, log_review_status, claim_cell.id
        )));
    }

    let mut result = append_cell_transition_morphism(
        &store_api,
        &replay.case_space,
        &packet.target.cell_id,
        packet.target.transition_to,
        Some(packet.completion.reason.as_str()),
        operation_gate,
        "casegraphen packet resume",
    )?;
    if let Some(object) = result["result"].as_object_mut() {
        object.insert("status".to_owned(), json!("completed"));
    }
    Ok(result)
}

fn packet_refusal(packet_path: &Path, error: impl std::fmt::Display) -> NativeCliError {
    NativeCliError::invalid(format!(
        "evidence packet {} was refused: {error}",
        packet_path.display()
    ))
}

//! GitHub issue-to-PR evidence adapter (issue #102).
//!
//! An **experimental, store-free, read-only** product surface: it observes a
//! GitHub issue→PR trajectory (review findings, CI checks, independence
//! classification) without lifting anything and without ever touching a
//! `NativeCaseStore`. GitHub, CI, review bots, and the implementation agent
//! are observation sources, never acceptance authorities — every record here
//! carries `accepted: false` (and the six computed records carry
//! `mutation_performed: false` at the CLI seam that wraps them).
//!
//! This module must not import `native_store`. That is the structural form
//! of the non-goal "GitHub is not a source of truth": the adapter cannot
//! reach the ledger at all. Mutation-capable follow-ups (attaching adapter
//! output to a case, accepting it, transitioning cells) go through the
//! existing gated commands with adapter outputs as ordinary artifacts.
//!
//! Trust boundary (see `model.rs` and design doc §6 for the full argument):
//! the only caller-authored input is [`model::CaptureManifest`], which is
//! strict-parsed and carries no trust vocabulary. The six record contracts
//! are outputs only, with one narrow, checked exception: `github refresh
//! --previous-observation` reads a [`model::PrObservation`] back as the
//! operator's declared review basis, and its content hash is recomputed and
//! checked before it is trusted.

mod independence;
mod model;
mod normalize;
mod projection;
mod refresh;

pub use independence::{
    classify_evidence_role, evaluate_independence, implementation_actor_ids, EvidenceSubject,
};
pub use model::*;
pub use normalize::{normalize, NormalizedCapture};
pub use projection::project_review;
pub use refresh::classify_refresh;

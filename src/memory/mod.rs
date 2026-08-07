//! Evidence-grounded, temporally governed project-memory proposals and views.
//!
//! This experimental surface derives read-only memory from an already replayed
//! [`CaseSpace`](crate::native_model::CaseSpace). It does not own persistence,
//! review, operation gates, model execution, or an authoritative index.

mod authority;
mod conflicts;
mod index;
mod model;
mod projection;
mod query;
mod temporal;
mod validation;

pub use index::{rebuild_memory_index, validate_memory_index};
pub use model::*;
pub use projection::query_memory;
pub use query::source_records_for_claim;
/// Re-exported for `github_evidence::normalize`: the manifest's
/// `captured_at` is the one caller-authored timestamp the adapter validates
/// (provider timestamps elsewhere stay verbatim, unvalidated strings).
pub(crate) use temporal::validate_timestamp;
pub use validation::{
    build_claim_proposal, parse_memory_claim, parse_memory_policy, parse_memory_query,
    parse_memory_source_record, parse_memory_use_report, validate_memory_claim,
    validate_memory_policy, validate_memory_proposal, validate_memory_query,
    validate_memory_source_record, validate_memory_use_report,
};

/// Returns the lowercase hexadecimal SHA-256 used by content-addressed memory artifacts.
pub fn content_hash(bytes: &[u8]) -> String {
    validation::sha256(bytes)
}

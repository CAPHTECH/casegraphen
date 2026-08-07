//! The embedded catalog behind `casegraphen schema list` / `schema get`.
//!
//! Issue #111: `install.sh` used to copy at most three schemas — the ones
//! three skills happened to need — out of `schemas/experimental/`, and
//! nothing at all out of `schemas/casegraphen/`, the 17-contract stable
//! surface. A consumer with only the installed binary and skills had no way
//! to obtain a schema a skill instructed them to author against, short of
//! cloning this repository. See ADR 0031 for why this crate now emits every
//! schema and example from the binary itself rather than shipping more
//! per-skill copies: one embedded source, not N drifting copies.
//!
//! Every `*.schema.json` and `*.example.json` file under `schemas/casegraphen/`
//! and `schemas/experimental/` is embedded with `include_str!` at compile
//! time, so the catalog can never diverge from the files it was built from —
//! there is no separate "did we remember to ship this one" step. `tests.rs`
//! in this module proves the catalog covers exactly the files on disk, so a
//! new schema file that is added without a matching `entry!` line fails the
//! build's tests, not a consumer's `schema get`.
//!
//! An entry's `id` is not hand-copied from the file either: it is read back
//! out of the embedded content itself (`$id` for a schema, the fixture's own
//! `schema` field for an example) the first time the catalog is built, so the
//! catalog's notion of a file's identity can never disagree with the file's
//! own declared identity.

use serde_json::Value;
use std::sync::OnceLock;

/// Whether a catalog entry comes from the strict `schemas/casegraphen/`
/// contract surface or the `schemas/experimental/` surface, which
/// `schemas/experimental/README.md` documents as free to break before
/// promotion. Carried through to both `schema list` and `schema get` so a
/// consumer can never mistake one for the other (issue #111's acceptance
/// criterion that the distinction survive).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SchemaStability {
    Stable,
    Experimental,
}

impl SchemaStability {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Experimental => "experimental",
        }
    }
}

/// One embedded file: its name as it appears under `schemas/`, whether it
/// came from the stable or experimental tree, its raw content, and the
/// identity the content itself declares. `id` is `None` only for a file that
/// fails to parse as JSON or carries neither field — `tests.rs` proves that
/// never happens for the files actually shipped.
pub(crate) struct SchemaCatalogEntry {
    pub(crate) file: &'static str,
    pub(crate) stability: SchemaStability,
    pub(crate) content: &'static str,
    pub(crate) id: Option<String>,
}

macro_rules! entry {
    ($dir:literal, $stability:expr, $file:literal) => {
        (
            $stability,
            $file,
            include_str!(concat!("../schemas/", $dir, "/", $file)),
        )
    };
}

/// `(stability, file, content)` for every embedded file. Sorted the way `ls`
/// lists them (stable tree, then experimental tree, each alphabetical) so a
/// diff against `ls schemas/*/*.{schema,example}.json` stays easy to read by
/// eye; `tests.rs` checks the set, not the order.
#[rustfmt::skip]
const RAW: &[(SchemaStability, &str, &str)] = &[
    entry!("casegraphen", SchemaStability::Stable, "case.graph.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "case.graph.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "case.report.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "coverage.policy.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "coverage.policy.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "evidence.packet.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "evidence.packet.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "execution.plan.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "execution.plan.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "execution.trace.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "execution.trace.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "github.issue-snapshot.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "github.issue-snapshot.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "native-cli.refusal.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "native-cli.refusal.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "native-cli.report.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "native.case.report.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "native.case.report.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "native.case.space.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "native.case.space.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "native.morphism-log-entry.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "native.morphism-propose-input.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "native.morphism-propose-input.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "operation-gate-profiles.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "operation-gate-profiles.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "projection.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "projection.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "worker.binding.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "worker.binding.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "worker.report.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "worker.report.schema.json"),
    entry!("casegraphen", SchemaStability::Stable, "workflow.graph.example.json"),
    entry!("casegraphen", SchemaStability::Stable, "workflow.graph.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "compiler.inputs.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "compiler.inputs.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "compiler.inputs.v1.example.json"),
    entry!("experimental", SchemaStability::Experimental, "compiler.inputs.v1.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "compiler.verification_performance_report.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.catalog.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.catalog.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.notification.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.notification.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.request.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.request.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.resource_projection.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.resource_projection.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.response.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.response.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.response.v0.success.example.json"),
    entry!("experimental", SchemaStability::Experimental, "deployment_bundle.migration_proposal.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "deployment_bundle.migration_proposal.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "deployment_bundle.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "deployment_bundle.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "deployment_policy_manifest.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "deployment_policy_manifest.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "execution_topology.review.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "execution_topology.review.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "execution.topology.file-review.example.json"),
    entry!("experimental", SchemaStability::Experimental, "execution.topology.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "execution.topology.worktree.example.json"),
    entry!("experimental", SchemaStability::Experimental, "expansion.policy.example.json"),
    entry!("experimental", SchemaStability::Experimental, "expansion.policy.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "git.worktree_record.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "git.worktree_record.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "github.capture_manifest.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "github.capture_manifest.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "github.check_evidence.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "github.check_evidence.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "github.pr_observation.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "github.pr_observation.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "github.refresh_result.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "github.refresh_result.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "github.review_finding.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "github.review_finding.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "github.review_independence.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "github.review_independence.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "github.review_projection.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "github.review_projection.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "graph_compiler.report.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "graph_lint.report.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "graph_simulation.report.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "graph_simulation.request.example.json"),
    entry!("experimental", SchemaStability::Experimental, "graph_simulation.request.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.expansion_evaluation_input.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.expansion_evaluation_input.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.memory_proposal_input.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.memory_proposal_input.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.memory_read_input.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.memory_read_input.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.proposal_compiler_input.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.proposal_compiler_input.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.resource_disposition_input.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.resource_disposition_input.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.resource_reconciliation_input.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.resource_reconciliation_input.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.resource_reservation_input.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.resource_reservation_input.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.reviewed_compiler_input.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.reviewed_compiler_input.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.streaming_run_input.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.streaming_run_input.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.verification_lineage_input.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "mcp.verification_lineage_input.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.claim.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.claim.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.claim_proposal.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.claim_proposal.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.index.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.index.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.policy.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.policy.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.projection.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.projection.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.query.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.query.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.relation_proposal.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.relation_proposal.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.source_record.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.source_record.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.use_report.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.use_report.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.allocator_checkpoint.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.allocator_checkpoint.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.allocator_compaction.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.allocator_compaction.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.allocator_configuration.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.allocator_configuration.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.allocator_event.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.allocator_event.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.allocator_retention_policy.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.allocator_retention_policy.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.declaration.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.declaration.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.rate_limit_capacity.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.rate_limit_capacity.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.reconciliation.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.reconciliation.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.reservation_disposition.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.reservation_disposition.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.reservation.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.reservation.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.reviewed_deployment_binding.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "resource.reviewed_deployment_binding.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.graph_expectation.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.graph_expectation.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.integration_report.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.integration.jsonl-record.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.integration.jsonl-record.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.node_report.example.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.node_report.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.resource_allocation.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.resource_allocation.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.resource_expectation_bundle.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.resource_expectation_bundle.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.stream_event.example.json"),
    entry!("experimental", SchemaStability::Experimental, "runtime.stream_event.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "skill.orchestration_handoff.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "skill.orchestration_handoff.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "streaming.reconciliation.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "streaming.reconciliation.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "topology.patch.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "topology.patch.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "topology.redesign_disposition_log.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "topology.redesign_disposition_log.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "topology.redesign_proposal.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "topology.redesign_proposal.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "verification_lineage_declarations.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "verification_lineage_declarations.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "verification.policy.example.json"),
    entry!("experimental", SchemaStability::Experimental, "verification.policy.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "verification.policy_result.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "verification.policy_result.v0.schema.json"),
];

/// Reads a file's own declared identity back out of its content: `$id` for a
/// `*.schema.json`, the fixture's own `schema` field for a `*.example.json`.
/// Never hand-copied, so the catalog cannot assert an identity a file does
/// not itself carry.
fn declared_id(file: &str, content: &str) -> Option<String> {
    let key = if file.ends_with(".schema.json") {
        "$id"
    } else if file.ends_with(".example.json") {
        "schema"
    } else {
        return None;
    };
    let value: Value = serde_json::from_str(content).ok()?;
    value.get(key)?.as_str().map(str::to_owned)
}

/// The full catalog, built once from `RAW` and cached: `declared_id` reparses
/// JSON, which is unnecessary work to repeat on every `schema list`/`schema
/// get` call within one process.
pub(crate) fn catalog() -> &'static [SchemaCatalogEntry] {
    static CATALOG: OnceLock<Vec<SchemaCatalogEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        RAW.iter()
            .map(|(stability, file, content)| SchemaCatalogEntry {
                file,
                stability: *stability,
                content,
                id: declared_id(file, content),
            })
            .collect()
    })
}

/// Looks up a schema's own JSON content by the string a `$ref` used to name
/// it. This repository's cross-file `$ref`s use two conventions — the
/// `casegraphen` tree names a literal filename
/// (`"native.case.space.schema.json#/..."`), the `experimental` tree mostly
/// names the target's own `$id`
/// (`"casegraphen.experimental.github.pr_observation.v0#/..."`) — so a match
/// on either is accepted rather than guessing which convention a given ref
/// uses. Cross-file `$ref`s in this repository never target an example, so
/// this only searches `*.schema.json` entries; filenames and ids are each
/// unique across the whole catalog (`tests.rs` proves both).
fn schema_content_by_target(target: &str) -> Result<Value, String> {
    let entry = catalog()
        .iter()
        .find(|entry| {
            entry.file.ends_with(".schema.json")
                && (entry.file == target || entry.id.as_deref() == Some(target))
        })
        .ok_or_else(|| format!("$ref names unknown schema {target:?} (tried filename and $id)"))?;
    serde_json::from_str(entry.content)
        .map_err(|error| format!("{target} does not parse as JSON: {error}"))
}

/// Resolves a JSON Pointer (RFC 6901), without its leading `#`, against
/// `document`.
fn resolve_pointer<'a>(document: &'a Value, pointer: &str) -> Result<&'a Value, String> {
    if pointer.is_empty() {
        return Ok(document);
    }
    let mut current = document;
    for raw_segment in pointer.trim_start_matches('/').split('/') {
        let segment = raw_segment.replace("~1", "/").replace("~0", "~");
        current = match current {
            Value::Object(map) => map
                .get(&segment)
                .ok_or_else(|| format!("pointer segment {segment:?} not found in {pointer:?}"))?,
            Value::Array(items) => {
                let index: usize = segment.parse().map_err(|_| {
                    format!("pointer segment {segment:?} is not an array index in {pointer:?}")
                })?;
                items
                    .get(index)
                    .ok_or_else(|| format!("pointer index {index} out of range in {pointer:?}"))?
            }
            _ => {
                return Err(format!(
                    "cannot descend into {segment:?}: not an object or array"
                ))
            }
        };
    }
    Ok(current)
}

/// Splits a `$ref` string into `(file, pointer)`. A ref with no file
/// component (`"#/..."`) resolves against `current_file`.
fn split_ref(reference: &str, current_file: &str) -> (String, String) {
    match reference.split_once('#') {
        Some(("", pointer)) => (current_file.to_owned(), pointer.to_owned()),
        Some((file, pointer)) => (file.to_owned(), pointer.to_owned()),
        None => (reference.to_owned(), String::new()),
    }
}

const MAX_REF_DEPTH: usize = 64;

/// Inlines every `$ref` that crosses a file boundary, so a schema served by
/// `schema get` is self-contained and validates with a bare `python3 -m
/// jsonschema` — no `--base-uri`, no local checkout. Issue #147 shipped the
/// first schema with a cross-file `$ref` (reusing case_morphism's property
/// definitions instead of duplicating them) without noticing that
/// `jsonschema`'s CLI cannot resolve a relative cross-file `$ref` at all
/// without a base URI a served, standalone document has no way to carry.
///
/// A `$ref` that stays inside `served_file` is left exactly as written:
/// same-document fragment refs already resolve correctly with no base URI,
/// and leaving them alone keeps every schema with no cross-file `$ref`
/// byte-identical to its source file, which
/// `get_by_id_returns_the_gate_profiles_schema_matching_the_source_file`
/// (`tests/schema_command.rs`) depends on. Once a `$ref` has crossed into a
/// foreign file, though, that file's own "local" refs are foreign to
/// `served_file` too — a `$ref` copied verbatim would dangle — so they get
/// inlined recursively all the way down.
fn dereference_cross_file_refs(
    served_file: &str,
    current_file: &str,
    value: Value,
    depth: usize,
) -> Result<Value, String> {
    if depth > MAX_REF_DEPTH {
        return Err(format!(
            "$ref depth exceeded {MAX_REF_DEPTH} while serving {served_file} (cycle?)"
        ));
    }
    match value {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                if reference.starts_with('#') && current_file == served_file {
                    return Ok(Value::Object(map));
                }
                if map.len() > 1 {
                    return Err(format!(
                        "{served_file}: a cross-file $ref ({reference}) has sibling keywords, \
                         which this resolver does not merge — give it no siblings or extend the resolver"
                    ));
                }
                let (target_file, pointer) = split_ref(reference, current_file);
                let target_document = schema_content_by_target(&target_file)?;
                let resolved = resolve_pointer(&target_document, &pointer)?.clone();
                return dereference_cross_file_refs(served_file, &target_file, resolved, depth + 1);
            }
            let mut resolved_map = serde_json::Map::with_capacity(map.len());
            for (key, member) in map {
                resolved_map.insert(
                    key,
                    dereference_cross_file_refs(served_file, current_file, member, depth + 1)?,
                );
            }
            Ok(Value::Object(resolved_map))
        }
        Value::Array(items) => items
            .into_iter()
            .map(|item| dereference_cross_file_refs(served_file, current_file, item, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        other => Ok(other),
    }
}

/// The content `schema get` serves for a `*.schema.json` entry: the file's
/// own JSON with every cross-file `$ref` inlined, so what a consumer
/// receives is self-contained. Not applied to `*.example.json` entries —
/// examples carry data, not `$ref` semantics, and this repository's
/// cross-file refs never target one.
pub(crate) fn served_schema_content(file: &str) -> Result<Value, String> {
    let content = schema_content_by_target(file)?;
    dereference_cross_file_refs(file, file, content, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, fs, path::Path};

    /// Lists the `*.schema.json` and `*.example.json` basenames actually
    /// present under `dir` on disk, independent of `RAW` — this is the
    /// ground truth `RAW` is checked against below.
    fn on_disk_contract_files(dir: &Path) -> BTreeSet<String> {
        fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
            .map(|entry| {
                entry
                    .expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.ends_with(".schema.json") || name.ends_with(".example.json"))
            .collect()
    }

    /// The completeness guard issue #111 asked for: every `*.schema.json`
    /// and `*.example.json` file under both schema trees has exactly one
    /// `entry!` line, and every `entry!` line names a file that still
    /// exists. A schema added to disk without a matching line here fails
    /// this test, not a consumer's `schema get` months later.
    #[test]
    fn catalog_matches_the_schema_trees_on_disk_exactly() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let expected: BTreeSet<String> =
            on_disk_contract_files(&manifest_dir.join("schemas/casegraphen"))
                .into_iter()
                .chain(on_disk_contract_files(
                    &manifest_dir.join("schemas/experimental"),
                ))
                .collect();

        let cataloged: BTreeSet<String> =
            RAW.iter().map(|(_, file, _)| (*file).to_owned()).collect();

        let missing_from_catalog: Vec<_> = expected.difference(&cataloged).collect();
        assert!(
            missing_from_catalog.is_empty(),
            "schema files on disk with no `entry!` line in schema_catalog.rs: {missing_from_catalog:?}"
        );

        let missing_from_disk: Vec<_> = cataloged.difference(&expected).collect();
        assert!(
            missing_from_disk.is_empty(),
            "`entry!` lines in schema_catalog.rs naming a file no longer on disk: {missing_from_disk:?}"
        );

        assert_eq!(
            RAW.len(),
            cataloged.len(),
            "a filename appears in more than one `entry!` line"
        );
    }

    /// Every `*.schema.json` file parses as JSON and declares `$id` — a
    /// silent `None` here would mean `schema get --id` can never reach that
    /// schema. `*.example.json` files are not held to this: a bare-record
    /// fixture such as `control_plane.catalog.v0.example.json` or
    /// `runtime.integration.jsonl-record.v0.example.json` legitimately has
    /// no top-level `schema` field of its own, and stays reachable through
    /// `schema get --file` instead.
    #[test]
    fn every_schema_declares_its_own_id() {
        for entry in catalog()
            .iter()
            .filter(|entry| entry.file.ends_with(".schema.json"))
        {
            assert!(
                entry.id.is_some(),
                "{} did not parse or had no declared $id",
                entry.file
            );
        }
    }

    /// `schema get --id` looks entries up by this field, so two files
    /// claiming the same schema identity would make the lookup
    /// non-deterministic without this ever being caught.
    #[test]
    fn schema_ids_are_unique_across_the_catalog() {
        let mut seen = std::collections::HashMap::new();
        for entry in catalog() {
            let Some(id) = &entry.id else { continue };
            // Only *.schema.json ids need to be unique: examples legitimately
            // share their owning schema's id (execution.topology.v0 has two).
            if !entry.file.ends_with(".schema.json") {
                continue;
            }
            if let Some(previous) = seen.insert(id.clone(), entry.file) {
                panic!("{} and {previous} both declare $id {id:?}", entry.file);
            }
        }
    }

    /// Collects every `$ref` string reachable from `value`.
    fn collect_refs(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                    out.push(reference.to_owned());
                }
                for member in map.values() {
                    collect_refs(member, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect_refs(item, out);
                }
            }
            _ => {}
        }
    }

    /// Issue #147 shipped a schema with a cross-file `$ref` and only later
    /// discovered that a bare `python3 -m jsonschema` — what a consumer
    /// actually runs, per `SKILL.md`'s own recipe — cannot resolve one
    /// without a base URI a served document has no way to carry. This is the
    /// regression guard for the fix: every `*.schema.json` entry, served the
    /// way `schema get` serves it, must contain no `$ref` naming a file
    /// other than itself. A same-document `$ref` (`"#/..."`) is fine — it
    /// needs no base URI — and is the only kind this should ever find, since
    /// `served_schema_content` inlines every other kind.
    #[test]
    fn served_schemas_carry_no_cross_file_ref() {
        for entry in catalog()
            .iter()
            .filter(|entry| entry.file.ends_with(".schema.json"))
        {
            let served = served_schema_content(entry.file)
                .unwrap_or_else(|error| panic!("serve {}: {error}", entry.file));
            let mut refs = Vec::new();
            collect_refs(&served, &mut refs);
            let cross_file: Vec<_> = refs
                .iter()
                .filter(|reference| !reference.starts_with('#'))
                .collect();
            assert!(
                cross_file.is_empty(),
                "{} still carries a cross-file $ref after serving: {cross_file:?}",
                entry.file
            );
        }
    }
}

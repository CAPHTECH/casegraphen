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
    entry!("experimental", SchemaStability::Experimental, "control_plane.response.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "control_plane.response.v0.schema.json"),
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
    entry!("experimental", SchemaStability::Experimental, "memory.claim.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.claim.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.index.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.index.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.policy.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.policy.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.projection.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.projection.v0.schema.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.query.v0.example.json"),
    entry!("experimental", SchemaStability::Experimental, "memory.query.v0.schema.json"),
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
}

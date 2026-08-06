---
name: casegraphen-integrate
description: Direct task skill for integrating generic JSONL reports and artifacts from an external runtime with a typed CaseGraphen execution topology. Use for integration-only validation of runtime.node_report.v0, completeness reconciliation, retry lineage, caller revision, and content-addressed artifacts; use casegraphen-orchestrate for multi-phase routing. Stops at unreviewed evidence/morphism proposals.
---

# Integrate an external runtime without accepting its claims

Use the host integration backed by `casegraphen::runtime_integration::GenericJsonlReconciler`.
This Skill does not schedule nodes, retry work, call models, or mutate a case.

## Workflow

1. Record the caller-observed `base_revision_id` and exact reviewed topology.
   Never replace the revision with a newer value.
2. Lint the topology with the real `casegraphen graph lint` command. Do not
   recreate graph rules in the Skill.
3. Read [generic-jsonl.md](references/generic-jsonl.md), then stream artifacts
   and `runtime.node_report.v0` envelopes into `GenericJsonlReconciler`.
4. Reconcile only through the library. It derives the strict runtime graph
   expectation from the canonical topology and delegates node completeness,
   terminal retry selection, parent lineage, edge handoffs, schema joins, and
   artifact-byte accounting to `runtime_protocol::reconcile_runtime_reports`.
   `complete` means both `node_complete` and `dataflow_complete`; do not call a
   set of independently successful nodes a completed graph.
5. If the result halts `incomplete_runtime_reports`, obtain missing reports or
   artifacts from the external runtime and ingest them. Do not synthesize one.
6. If the result halts `resource_reconciliation_incomplete`, correct or obtain
   the topology-bound declaration, host-issued reservation, and typed
   allocation records. For the operational host, submit them as an exact
   `runtime.resource_expectation_bundle.v0`; do not infer a grant from the
   compact node-report allocation summary.
7. If the result halts `needs_review`, present every proposal at the review
   seam. Every proposal remains `unreviewed`; `accepted` remains false.

For standalone operation, use the inventory-governed MCP tools:
`compile_deployment_bundle`, `compile_reviewed_deployment_bundle`, `reconcile_run`, `reserve_resources`, `release_resources`,
`reconcile_resources`, `reconcile_streaming_run`, and
`reconcile_verification_lineage`. They delegate to the same
canonical modules described above. Reservation and disposition require an
explicit observed revision and caller-declared audit context. Existing active
reservations, dispositions, and rate capacities are host-canonical allocator
state and must never be supplied by the caller. The MCP bearer token authorizes host
access; the audit context is attribution only and is never a CaseGraphen
operation gate. Streaming re-derives canonical readiness and resource permits
for the exact current revision. In v0, `streaming` is a compatibility name for
`terminal_artifact_stage_pipelining_v0`: a proposal requires the canonical
terminal producer and final byte-observed artifact. Never claim chunk-level
producer/consumer overlap from this result.

Use `compile_deployment_bundle` only for an unreviewed proposal. A deployment
that will reserve resources must first be accepted by the dedicated topology
review path, then compiled with `compile_reviewed_deployment_bundle`. Supply
only the case-space and claim identities plus the observed accepted revision;
never construct a compilation mode or deployment authority. Reservation must
name the resulting content-addressed bundle and retain the allocator-journal
review binding in the resource expectation bundle.
The persisted bundle must include canonical `compiler.inputs.json`. The host
recompiles those untrusted retained inputs and compares every output byte
before deriving deployment authority; hash-consistent hand-built artifacts are
not compiler provenance.

## Non-negotiable boundary

- A runtime-reported success, model, context, verifier, time, cost, allocation,
  worktree, or commit is an untrusted declaration.
- Preserve runtime producer/verifier lineage only as
  `verification_lineage_declarations.v0`. Never pass a declared actor,
  capability, disposition, or quorum as a ledger-derived proof. The strong
  policy path requires opaque proofs derived by CaseGraphen from the exact
  observed current ledger, canonical gate/capability cells, historical
  dispatch/review morphisms, and matching report/trace bytes. Producer and
  verifier proofs join on the execution trace's shared subject revision; they
  need not have been recorded by the same ledger morphism or revision. Strong
  reconciliation must replay the current case space so retained proofs are
  invalidated by review reopen/reject or capability invalidation.
- For the shipped shell-worker workflow, retain `worker.report.json`,
  `execution.trace.json`, `stdout`, and `stderr`, then derive the producer with
  `derive_native_cli_run_producer_proof`. After an independent canonical CLI
  review, derive the verifier with
  `derive_native_cli_review_verifier_proof`. A normal review is already a
  content-bound review execution record; never fabricate a verifier
  `ExecutionTrace`. See
  [the verification-lineage guide](../../docs/guides/verification-lineage.md).
- Without custom Rust, call `reconcile_verification_lineage` with the exact
  current revision, claim cell, retained artifact-root-relative report/trace/
  stdout/stderr paths, canonical review morphism IDs, and policy. The host
  derives opaque proofs internally and returns only the read-only policy
  result. Never repeat a review ID to satisfy quorum or expect proof objects in
  the response.
- Never turn an ingest report into accepted evidence or apply its morphism.
- Never infer retry lineage from line order; only explicit
  `retry_of_attempt_id` is authoritative for reconciliation.
- Every data edge must carry one observed content-addressed artifact from the
  canonical terminal source output into the canonical terminal target input.
  Do not infer a handoff from node success or matching schema names alone.
- Never silently omit an invalid line, hash mismatch, duplicate attempt, or
  missing node.
- Replaying the same valid envelope is idempotent; an identifier naming
  different content is a refusal finding.

---
name: casegraphen-integrate
description: Integrate generic JSONL reports and artifacts from an external runtime with a typed CaseGraphen execution topology. Use when validating runtime.node_report.v0, reconciling completeness, preserving retry lineage and caller revision, content-addressing artifacts, and stopping at unreviewed evidence/morphism proposals.
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
4. Reconcile only through the library. It delegates completeness, schema
   matching, graph joins, missing reports, artifact accounting, and retry
   lineage to `runtime_protocol::reconcile_runtime_reports`.
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
`compile_deployment_bundle`, `reconcile_run`, `reserve_resources`, `release_resources`,
`reconcile_resources`, and `reconcile_streaming_run`. They delegate to the same
canonical modules described above. Reservation and disposition require an
explicit observed revision and caller-declared audit context. Existing active
reservations, dispositions, and rate capacities are host-canonical allocator
state and must never be supplied by the caller. The MCP bearer token authorizes host
access; the audit context is attribution only and is never a CaseGraphen
operation gate. Streaming re-derives canonical readiness and resource permits
for the exact current revision.

## Non-negotiable boundary

- A runtime-reported success, model, context, verifier, time, cost, allocation,
  worktree, or commit is an untrusted declaration.
- Never turn an ingest report into accepted evidence or apply its morphism.
- Never infer retry lineage from line order; only explicit
  `retry_of_attempt_id` is authoritative for reconciliation.
- Never silently omit an invalid line, hash mismatch, duplicate attempt, or
  missing node.
- Replaying the same valid envelope is idempotent; an identifier naming
  different content is a refusal finding.

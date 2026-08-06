---
name: casegraphen-memory-query
description: Direct task skill for querying CaseGraphen's experimental, evidence-grounded project memory as a read-only, revision-bound projection. Use for memory-query-only requests for constraints, decisions, procedures, failure patterns, sources, conflicts, or history; use casegraphen-orchestrate for multi-phase routing. Never mutates accepted state.
---

# Query governed project memory

Use the Memory Plane as a read-only projection of an exact replayed CaseSpace.
It is not a conversation store, and relevance is not authority.

## Workflow

1. Re-read the case and record its exact `current_revision_id`. Never substitute
   a newer revision into an old query.
2. Select an operator-owned `memory.policy.v0` whose actor grant explicitly
   permits the requesting actor, audience, purpose, project, sensitivity, and
   authority ceiling. Do not author a broader grant merely to make a query pass.
3. Build `memory.query.v0` with that exact `base_revision_id`, a canonical UTC
   `as_of`, narrow scope and memory kinds, and bounded item/token budgets.
4. Use `casegraphen memory query --store <dir> --case-space-id <id> --input
   <query.json> --policy <policy.json> --format json`. The operational MCP
   equivalent is `memory_query`.
5. Before acting, inspect `selected_claim_ids`, item `status`, `source_refs`,
   `authority`, `valid_time`, `hard_conflict`, `omissions`, and `losses`. Require
   `mutation_performed: false` and the exact base revision. For MCP responses,
   also require `read_only: true` and `accepted: false`.
6. Use `memory_explain` / `casegraphen memory explain` for one claim and
   `memory_sources` / `casegraphen memory sources` to obtain its immutable
   source references. Escalate to source bytes when a decision depends on a
   condition a structured claim may have compressed.
7. Use `memory_conflicts` before high-risk changes. Use `memory_history` only
   for an explicitly historical question; superseded, expired, retracted,
   rejected, and not-yet-valid items are not current instructions.

## Boundary

- `memory_query`, `memory_explain`, `memory_history`, `memory_conflicts`, and
  `memory_sources` never authorize or perform a mutation.
- Never treat a projection, similarity score, confidence, repeated source, or
  Memory Use Report as an operation gate.
- Never remove conflict, omission, loss, provenance role, authority, or valid
  time when converting a projection into agent context.
- On `stale_revision`, replay and deliberately build a new query. Do not retry
  the old query with a silently substituted revision.
- Do not call `review`, `morphism apply`, `evidence attach`, `run`, or `operate`
  from this Skill.

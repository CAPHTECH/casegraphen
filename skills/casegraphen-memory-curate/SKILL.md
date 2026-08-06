---
name: casegraphen-memory-curate
description: Direct task skill for turning immutable project sources into strict, unreviewed CaseGraphen Memory Claim and supersession/retraction/procedure proposals. Use for memory-curation-only work preserving valid time, scope, provenance role, authority ceiling, and the independent review seam; use casegraphen-orchestrate for multi-phase routing.
---

# Curate memory proposals

Produce proposals only. The proposer does not accept, review, apply, or persist
managed CaseGraphen state.

## Workflow

1. Capture the exact source bytes before extraction. Author a
   `memory.source_record.v0` with its SHA-256, capture time, origin actor,
   source boundary, authority origin, sensitivity, and artifact reference.
2. Run `casegraphen memory source inspect --source-record <source.json>
   --source-artifact <bytes> --format json`. `memory source attach` emits the
   same content-addressed proposal boundary but does not persist or mutate.
3. Extract only reusable project observations, facts, constraints, decisions,
   procedures, failure patterns, goals, preferences, commitments, authority
   statements, or references. Leave tokens, retries, and tool-call detail in
   raw traces.
4. Author `memory.claim.v0`. Preserve subject, narrow project/case/actor scope,
   valid time, exact artifact source refs, derivation actor/method, provenance
   role, sensitivity, and the lowest defensible authority ceiling. Keep
   `model_assertions_are_untrusted: true`; the contract has no acceptance field.
5. Run `casegraphen memory check --input <claim.json> --source-record
   <source.json> --source-artifact <bytes> --policy <policy.json> --format
   json`. Resolve hash, source, time, scope, and authority findings without
   weakening the policy.
6. Run `casegraphen memory propose --store <dir> --case-space-id <id> ...
   --format json`, or use the proposal-only MCP tools `memory_propose_claim`
   and `memory_propose_procedure`. The claim scope must name that replayed
   CaseSpace. Confirm proposed lifecycle, unreviewed status, `accepted: false`,
   and `mutation_performed: false`.
7. Never overwrite an old claim. Use `memory_propose_supersession` or
   `memory_propose_retraction`, naming the exact target claim and replayed base
   revision. Their relation proposals remain unreviewed.
8. Hand the proposal to an independent operator. Any later acceptance must use
   the existing review and gated morphism workflow outside this Skill.

## Boundary

- Summarization cannot raise authority. An accepted hard `authorized_by`
  binding from a suitable reviewer is required for elevation.
- Confidence and source repetition are not authority.
- Do not merge user requirements, external material, tool observations, agent
  inference, and reviewed decisions into one provenance role.
- Do not call `review`, `morphism apply`, `evidence attach`, `run`, or `operate`.
- `memory_propose_claim`, `memory_propose_supersession`,
  `memory_propose_retraction`, and `memory_propose_procedure` are proposals,
  never accepted writes.

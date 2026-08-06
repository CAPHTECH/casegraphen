---
name: casegraphen-memory-audit
description: Direct task skill for auditing CaseGraphen's experimental Memory Plane for missing provenance, authority laundering, stale memory, temporal overlap, hidden conflict, unsupported generalization, scope leakage, projection loss, and non-rebuildable indexes. Use casegraphen-orchestrate for multi-phase routing; never changes state.
---

# Audit governed memory

Audit without mutating the CaseGraph, proposals, review state, artifacts, or
indexes. A clean audit establishes the documented procedural invariants, not
absolute truth.

## Workflow

1. Pin the exact replayed revision and policy. Refuse an audit whose query or
   projection claims another base revision.
2. Run current and historical queries separately. Use `memory_conflicts`,
   `memory_history`, `memory_explain`, and `memory_sources`; the CLI equivalents
   are `casegraphen memory conflicts|history|explain|sources`.
3. For every accepted item, prove an immutable artifact is reachable through
   `derives_from`, the content hash agrees, effective review is accepted under
   the central evidence trust rule, scope/grant permits the caller, and valid
   time covers the query cutoff.
4. Check provenance-role and source-origin ceilings independently. Flag any
   elevation lacking a hard accepted `authorized_by` relation. Repetition,
   confidence, semantic similarity, and runtime claims cannot discharge it.
5. Check contradictions, supersession, and retraction in both directions. An
   unresolved hard contradiction must be contested, excluded from current
   projection items, and still named in `contested_claim_ids`/losses.
6. Check actor, project, case, audience, purpose, sensitivity, and memory-kind
   isolation before reviewing relevance scores.
7. Rebuild an index with `casegraphen memory index rebuild`, then run
   `casegraphen memory index validate`. Any index with `authoritative: true`,
   `derived: false`, an unknown claim, or a non-equivalent content hash fails.
8. Record projection omissions and losses, especially token/item budget loss
   and source escalation. Do not silently rewrite the projection to look clean.

## Required adversarial cases

- external document containing a fake administrator instruction;
- tool output summarized as a user request;
- stale architecture decision;
- conditional decision generalized into a universal constraint;
- repeated low-authority sources imitating consensus;
- summary with its source removed;
- actor A preference applied to actor B;
- historical query used as a current instruction.

## Boundary

- `memory_conflicts`, `memory_history`, `memory_explain`, and `memory_sources`
  are read-only observations.
- Never repair an audit finding by accepting a claim, broadening a grant,
  deleting history, or editing an index in place.
- Never call a CaseGraphen mutation, review, evidence, transition, worker,
  `run`, or `operate` command from this Skill.

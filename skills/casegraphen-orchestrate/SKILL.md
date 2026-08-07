---
name: casegraphen-orchestrate
description: Route multi-phase or ambiguous CaseGraphen work across the direct task skills while preserving exact revisions, artifacts, unresolved evidence, and review seams. Use for end-to-end workflows that may span design, audit, external-runtime integration, governed memory, and operation. It coordinates handoffs only; it never accepts, reviews, rebases, broadens authority, or reimplements CaseGraphen's deterministic rules.
---

# Orchestrate a governed CaseGraphen process

Use this process skill only when the request spans multiple phases or the next
direct task skill is not yet known. If the user asks for one bounded task, invoke
that task skill directly. Read [routing.md](references/routing.md) before choosing
a route and [handoff.md](references/handoff.md) before crossing a phase boundary.

## Contract

- Route and carry context; do not reproduce graph lint, completeness, retry,
  review, gate, temporal, authority, or acceptance rules.
- Never review or accept a proposal, silently rebase a stale revision, enable a
  worker, widen scope, or grant authority. These are explicit return seams.
- Automatic continuation is allowed only between read-only or proposal-only
  phases when the handoff is complete and no review or authority seam is open.
- Every boundary emits `skill.orchestration_handoff.v0`, validated against
  the schema `casegraphen schema get --id casegraphen.experimental.skill.orchestration_handoff.v0 --format json`
  returns.
- The handoff records facts observed from tools separately from unresolved
  evidence and runtime-declared claims. It never turns either into accepted state.

## Workflow

1. Classify the request as a direct task or a multi-phase process. For a direct
   task, name the selected task skill and stop orchestrating.
2. Pin the exact case-space id and observed revision when available. Inventory
   topology, reports, artifacts, content hashes, and unresolved evidence without
   inventing missing values.
3. Select exactly one next task skill using the routing table. State why it owns
   the phase and which responsibilities remain outside it.
4. Run or hand off to that skill. Preserve its outputs as artifacts; do not
   reinterpret a proposal, report, or runtime declaration as accepted evidence.
5. Emit a strict handoff using the example
   `casegraphen schema get --file skill.orchestration_handoff.v0.example.json --format json`
   returns.
6. If `return_required` is true, return to the named human/policy/authority seam.
   Otherwise continue only when `next_action.kind` is `invoke_task_skill`.
7. Finish only after the selected task skill reports its own completion and the
   handoff contains no unresolved required evidence or open seam.

## Representative routes

```text
native:
  intake -> casegraphen-design -> topology_review -> casegraphen-operate

external runtime:
  intake -> casegraphen-design -> topology_review -> external runtime
         -> casegraphen-integrate -> casegraphen-audit -> evidence_review
         -> casegraphen-operate
```

Memory query, curation, and audit use the same routing and seam rules. Memory
proposals still require the existing independent review and gated operation path.

## Stop and return

Return rather than continue when any of these is true:

- topology, evidence, plan, or memory review is required;
- the observed revision is stale or missing for a mutation;
- worker enablement, capability, credential, scope, or authority must change;
- a hard conflict or required evidence remains unresolved;
- a task skill refuses, or the requested next action is outside its boundary.

The process skill coordinates the lifecycle. `casegraphen-operate` remains the
single task skill that owns the shared revision, operation-gate, mutation, and
refusal protocol; it is intentionally not split.

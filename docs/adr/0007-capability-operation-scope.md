# ADR 0007: Decide Whether A Capability Is Scoped To An Operation

## Status

Proposed on 2026-07-31. Raised by finding 1 of
`docs/audit/authorization-and-evidence-coverage-2026-07-31.md`. Nothing is
implemented; this records the decision that has to be made and what each answer
costs.

## Context

The operation gate validates five things about a capability id: that it resolves
to a case cell, that the cell is `custom:capability`, that its lifecycle is
`active` or `accepted`, that its provenance is `accepted`, and that
`metadata.actor_ids` names the acting actor (`src/native_review.rs:298`). It
does not relate the capability to the operation being performed, because a
capability cell has no field that would say. The operation string is validated —
`gate.operation` must be the one the command mints — but never against the
capability.

The consequence is reproduced in the audit: `actor:release-runner`, holding only
the walkthrough's dispatch capabilities, performed `review accept`.

This matters more here than in most systems, because capability cells are the
authorization trust root. They enter only at lift/import and there is no
post-genesis path to grant, amend, or revoke one, so the grant an operator makes
at genesis is permanent. An operator writing four capability cells with four
distinct titles is stating four distinct authorities, and the tool is treating
them as one.

The documentation had drifted furthest: `authoring.md` told authors to separate
the roles "so a compromised runner cannot approve its own work". `f6a5c5e`
replaced that with what the tool actually does — the actor is the boundary, the
capability is not — so no one is currently operating on a false promise. That
correction is a holding action, not the answer.

## Decision

Not yet made. The options, with what each costs:

**(a) Scope capabilities to operations.** Add `metadata.operations` to the
capability cell — a list of the operation strings from the per-command table in
`docs/security/worker-execution-policy.md` — and have `check_operation_gate`
require `gate.operation` to appear in the union of the presented capabilities'
lists.

- It is a contract change under `schemas/casegraphen/`, so the `contract-change`
  skill applies and the field's shape is a contract decision.
- Capability cells enter only at genesis, so **every existing case space would
  have to be lifted again** to carry the new field. Whether the field is
  required or optional decides that: optional-and-absent would have to mean
  something, and "absent means all operations" reintroduces the defect by
  default while "absent means none" bricks existing spaces.
- It makes the four-cell split in the walkthrough mean what its titles say.

**(b) State that capabilities are not scoped.** Keep one capability as "this
actor may perform gated operations in this space", say so in the policy's
section 2.2, and treat the actor as the only authorization boundary.

- Costs nothing to implement and matches today's behaviour.
- It makes the per-command operation table in the policy purely descriptive: it
  says which operation string is recorded, not who may perform it.
- Whoever splits capability cells by role has to be told the split is
  documentation, not enforcement — the audit trail records which capability was
  named, and that is its whole value.

**(c) Something narrower**, such as scoping only the operations that promote
trust (`review`, `plan-review`, `morphism-apply`) and leaving the rest unscoped.
This buys most of (a)'s value at less contract churn, at the cost of a rule that
has to be justified case by case rather than read off the cell.

## Consequences

Undecided until this ADR is accepted. Two things hold in the meantime:

- The gate's other five checks are enforced and were re-verified during the
  audit; this is not a hole in the gate, it is a missing dimension of it.
- Finding 2 of the same audit lets a hard evidence requirement be cleared with
  no review at all, by an actor whose capability is unquestionably the right one
  for the operation it names. Scoping capabilities does not touch that, and
  fixing this ADR's question first would leave the larger hole open while
  looking like the authorization model had been hardened.

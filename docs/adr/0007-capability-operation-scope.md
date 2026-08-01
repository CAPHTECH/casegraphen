# ADR 0007: Decide Whether A Capability Is Scoped To An Operation

## Status

Accepted on 2026-08-01, as option (a) with "absent means none". Raised by
finding 1 of `docs/audit/authorization-and-evidence-coverage-2026-07-31.md`, and
implemented after the evidence-coverage class it defers to was closed.

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

**(a), with "absent means none."** `check_operation_gate` requires the operation
it is performing to appear in `metadata.operations` of one of the presented
capabilities. The cost that made (b) attractive — a contract change — did not
exist, and the cost that remains is one every operator here already pays
whenever a grant changes.

The options as they were weighed:

**(a) Scope capabilities to operations.** Add `metadata.operations` to the
capability cell — a list of the operation strings from the per-command table in
`docs/security/worker-execution-policy.md` — and have `check_operation_gate`
require `gate.operation` to appear in the union of the presented capabilities'
lists.

- **It is not a schema change.** `metadata` is `{"type": "object"}` in
  `native.case.space.schema.json`, and the gate already reads
  `metadata.actor_ids` out of it (`src/native_review.rs:341`). This is the
  second row of the `contract-change` decision table — an existing escape hatch
  the reducer already reads — so it costs a documented convention, not a new
  `$id`. That is much cheaper than this ADR first assumed.
- The real cost is what an absent field means. "Absent means every operation"
  keeps today's behaviour for every existing space and buys nothing by default.
  "Absent means none" is the enforcing choice and stops every existing space at
  its next gated command. Capability cells enter only at genesis and there is no
  amendment path, so the fix for an existing space is to lift it again — which
  is already how any capability change works here.
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

## Why (a), and why not first

(b) was attractive only while option (a) looked like a contract change with a
re-lift attached. It is not one. What (b) actually buys is that nothing breaks,
and what it costs is permanent: the per-command operation table in the policy
becomes descriptive, the four capability cells in the shipped walkthrough become
four labels for one authority, and every future reader of `authoring.md` has to
be told that the separation they are being asked to model is documentation. A
tool whose stated purpose is that "CaseGraphen decides; an LLM only proposes"
should not have an authorization root that records intent it does not enforce.

(c) — scoping only the trust-promoting operations — is the option to fall back
to if "absent means none" proves too disruptive in practice. It is strictly
weaker and needs a case-by-case justification for each operation left unscoped,
which is exactly the kind of rule that drifts.

**It should not be implemented first.** Finding 2 of the same audit lets a hard
evidence requirement be cleared with no review at all, by an actor whose
capability is unquestionably the right one for the operation it names. Scoping
capabilities does not touch that. Doing this one first would leave the larger
hole open while making the authorization model look hardened — and the audit's
own conclusion is that partial fixes in this area cost more in false assurance
than they buy in coverage.

## Consequences

The audit's finding 1 no longer reproduces: the walkthrough's dispatch-only
runner is refused with `capability capability:release-dispatch does not
authorize operation review`, and the release manager performs the same review
with the capability whose title says so. `authoring.md` states the enforced rule
again, having been corrected in `f6a5c5e` to stop promising it.

Every shipped fixture carrying capability cells was lifted with the field —
`schemas/casegraphen/native.case.space.example.json` and the walkthrough genesis
— and a case space written before this must be lifted again, which is how any
capability change works here. There is no migration path and deliberately no
permissive default: a space whose capabilities carry no `operations` authorizes
nothing, and fails at its next gated command with a message naming the field.

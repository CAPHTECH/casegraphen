# ADR 0015: A Packet Pauses At The Actor Seam

## Status

Accepted on 2026-08-01. Resolves issue #19, which asked for a packet layer
driving attach-then-transition and required this decision before an
implementation because the obvious shape — one command that does both —
trades a workflow convenience for a trust hole.

## Context

An operator wants to hand a runtime one file describing "here is the claim and
its artifacts, and here is what to do once it's accepted" and have the tool
carry it through: attach the evidence, wait for review, then transition the
target cell. The naive version is one command that does all three steps in one
invocation.

One command means one operation gate, which means one actor. If that one
invocation also performed the review, the actor that proposed the claim would
be the actor that accepted it — self-review, indistinguishable in the log from
an actual independent check. This project already hardened evidence review
once (the `evidence_boundary` work): a caller-declared trust value is not a
trust value. A packet that reviewed its own claim would reintroduce the same
hole one level up, in the shape of a convenience feature instead of a caller
argument.

The three steps also do not have equal weight. Attach and transition are
mechanical: read a file, hash it, mint a morphism. Review is not mechanical —
it is the one step where the tool must simply wait for a human or delegated
decision it did not make and cannot manufacture.

## Decision

**A packet never performs a review. `packet apply` always pauses after the
attach; `packet resume` refuses to transition until an independent review has
already landed in the log.**

- `packet apply` reads the packet's claim and artifacts and calls the exact
  same internal function `evidence attach` calls to build and append one
  `EvidenceAttach` morphism — the same forced-inferred/hash/refusal pipeline,
  the same `--satisfies` coverage rule, the same batching of claim and
  artifacts into one morphism. Its gate operation is `evidence-attach`, the
  attach's own operation string; a packet introduces no new operation
  vocabulary to authorize. It reports `paused_for_review` and (ADR 0016) a
  typed `needs_review` halt object, whose `next_operations` names the
  argument fields of the concrete `review accept` and `packet resume` calls
  to make next — target id, base revision, completed-through revision — so
  the pause is not silent. `packet apply` is one producer of that shared
  shape, not a second ad hoc copy beside it: `completed_through` and
  `next_operations` live once, inside `halt`, not duplicated at the top
  level. These are structured values, not assembled command strings: a
  packet's `claim.id` is attacker-controlled text, and interpolating it into
  a shell string an operator is told to paste would let one `claim.id` value
  inject extra flags into the very `review accept` this pause exists to keep
  independent.
- `packet resume` calls the exact same internal function `cell transition`
  calls, after three checks none of which it re-derives:
  - the claim cell must be `cell_type: evidence` — nothing else has a review
    status this rule speaks for.
  - the claim must be the evidence *this packet's own apply* attached: the
    log entry at exactly the named `--completed-through` revision must be an
    `EvidenceAttach` morphism whose `added_ids` contain the claim id. Without
    this, a packet could name a different, already-accepted attach's claim
    id and ride that accept to authorize a transition its own claim was
    never reviewed for.
  - the claim's **log-derived review status** — read via
    `latest_evidence_review_status`, with no fallback to the cell's own
    stored `provenance.review_status` — must be `accepted`. This is
    deliberately not the same function the findings section uses
    (`effective_evidence_review_status`, which legitimately falls back to
    the stored status for reporting): on a path that authorizes a durable
    mutation, "no review in the log" must never read as accepted, which a
    stored-status fallback would have let a genesis-authored, already-trusted
    evidence cell satisfy with no review morphism anywhere in the log.
  - `--completed-through <revision-id>` names the revision `packet apply`
    produced, and must appear in the replayed history.
  Its gate operation is `cell-transition`, for the same reason apply's is
  `evidence-attach`, and is validated before the packet file or any of the
  above is read — the same ordering `evidence attach` documents.
- Because apply's gate and resume's gate are each checked independently, they
  can be — and for the pause to mean anything, should be — held by different
  actors. Nothing stops the same actor from holding both capabilities, but the
  tool does not assume it: the review step is authorized exactly like every
  other `review accept`, through the operation gate naming `review`, which a
  packet never requests.

**`--completed-through` is an assertion, not a lookup**, for the same reason
`--base-revision-id` is (ADR 0008) and `--supersede-trace` is (ADR 0014): the
operator names the fact they are relying on, and the tool checks it against
what actually happened rather than inferring it from the graph's current
shape. Absence from history is a tool failure — a stale store, a rollback, or
the wrong space — not a rebase the tool should paper over by picking "the
latest revision that looks like it".

**The command namespace is `packet`, not `workflow`.** ADR 0003 converged the
workflow evaluator family into lift input and closed that namespace; reopening
it for a new feature would undo that decision by a side door. `packet` names
what the input is, the same way `evidence`, `cell`, and `plan` do.

**The packet input is strict JSON on the existing parse path, not a new
format.** Two contracts already justify this:

- ADR 0006's dependency criterion: a YAML parser is a new transitive dependency
  tree to audit for a feature that JSON already covers. It would remove no
  risk this project carries today and add a parser's worth of surface area.
- ADR 0010's strict-parse diagnostics: `schemas/casegraphen/*.schema.json`
  strictness plus `serde_path_to_error` is the one diagnostic path every input
  in this crate gets, including the location-qualified "which closed object
  rejected the field" report. A second input format would need its own
  diagnostic story or ship with a worse one.

A packet is therefore validated the same way a genesis snapshot or a morphism
proposal is: `additionalProperties: false` throughout, deserialized through
`parse_strict`, and its `claim` field is the identical `CaseCell` shape
`evidence attach` already reads — not a packet-specific evidence format.

## Why not the alternatives

**One command, one gate, both attach and transition, with a `--reviewed-by`
flag the caller fills in after reviewing out of band.** This does not remove
the self-review hole; it hides it. The flag still lets the same invocation's
gate stand in for the review the flag claims happened, exactly the
caller-declared trust value this project already refused once for
`evidence_boundary`.

**A `packet review` subcommand that performs the review as a third pause-free
step, gated separately within the same command.** A single process can still
hold every capability a store's gate profile lets it hold, so a
"separately gated step" inside one invocation is not separated by anything the
tool can check — it is the number of `--capability-id` flags on one command
line, not two actors. The only boundary the tool can actually observe is two
separate invocations, which is what `apply` then `resume` already are.

## Consequences

- `packet apply` and `packet resume` are two commands, not one, and resume can
  be run by a different process, a different day, under a different actor's
  gate, without the packet file changing.
- `evidence_attach` and `cell_transition` in
  `src/native_cli/ops/mutations.rs` are each split into a morphism-building
  function and an append function shared between the plain CLI command and the
  packet command, so a hardening pass to either rule lands on both callers at
  once.
- A packet that never gets reviewed leaves its claim attached and unreviewed
  forever, and `packet resume` refuses forever — the same shape as any other
  unreviewed evidence cell, not a new kind of stuck state.
- `docs/specs/casegraphen-native-case-management.md`, `src/cli_usage.txt`, and
  `README.md` all carry the two commands, and `tests/cli_surface.rs` enforces
  that they do.

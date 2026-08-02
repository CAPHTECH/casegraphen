# ADR 0018: A Retry Names What It Retries

## Status

Accepted on 2026-08-02. Resolves issue #33.

## Context

An operator complaint prompted issue #24: *"every worker failure adds another
trace and evidence record, so the history gets noisy."* #24 shipped trace
folding and it does not address that complaint, for a reason worth writing down
rather than working around.

The fold collapses a supersession chain, sourced from a trace's own
`metadata.superseded_trace_ids` — the relationship ADR 0014 records when an
operator asserts a started dispatch is dead. The noisy case is different: a step
fails, is retried with `--retry-step`, and fails again. Those entries cannot be
folded, and the rule that forbids it is one #24 imposed deliberately: *collapse
only what `superseded_trace_ids` explicitly names, never entries that are merely
adjacent or merely share a step id.*

That rule is right. A failed trace and its retry successor share `step_id`,
`plan_id`, and `work_cell_id` — three facts equally true of any two independent
dispatches of the same step, including causally unrelated ones. Folding on them
would invent a relationship the log does not record.

So the finding is not a rendering gap. **The retry never asserts a link to what
it retries.** `--retry-step` is an explicit operator act (ADR 0004, and ADR 0016
made `operate` refuse the flag so it stays one), but the act leaves no trace of
*what decision it was*. The ledger records that a second dispatch happened; it
does not record that it happened **because** the operator decided to retry the
first. "Was this a fresh dispatch or a retry of that one" is exactly the kind of
question this ledger exists to answer, and today it cannot.

The fact is not missing because it is hard to obtain. `select_steps`
(`src/native_cli/ops/run.rs`) already computes it: a step is ineligible when it
has a failed trace, and eligible again only when `--retry-step` names it. The
gate consults the failed traces and then discards them into a boolean.

## Decision

**A dispatch that was authorized past one or more failed traces records their
ids in `metadata.retried_trace_ids`.**

Three constraints on what that means.

1. **The tool computes it; it is never accepted from input.** The operator names
   a *step*. The tool resolves, from the log at selection time, which failed
   traces of that step the eligibility gate consulted, and writes those ids. A
   caller-supplied `retried_trace_ids` is a caller-declared trust value and is
   refused, on the same rule that forces evidence boundaries and content hashes.
2. **It records an existing decision; it does not add one.** The set written is
   exactly the set `select_steps` already tests. Nothing new is inferred, and in
   particular nothing is inferred from `step_id` adjacency — the failure #24's
   fold rule exists to prevent stays prevented.
3. **A retry is not a supersession, and the two must remain distinguishable.**
   `superseded_trace_ids` means *an operator asserted that dispatch was dead*
   (ADR 0014). `retried_trace_ids` means *that dispatch failed and an operator
   consented to another attempt*. They are different facts about different
   situations, and `decide_superseded_traces` already refuses to let
   `--supersede-trace` name an already-failed trace. Any rendering that folds
   both must say which relationship it folded; collapsing them into one
   "attempts" count would lose exactly what this ADR adds.

### Where the data goes, and why there is no schema change

Under the `contract-change` skill's first step, this is the second row: data a
reader consumes structurally, on a record that already has a `metadata` escape
hatch the code reads. `ExecutionTrace.metadata` is `{"type": "object"}` in
`schemas/casegraphen/execution.trace.schema.json`, and
`metadata.superseded_trace_ids` is the established precedent — read by
`native_cli_text.rs::superseded_trace_ids`, written by the run path. Retry
lineage extends that convention rather than opening a second one, which is what
"a decision rule has exactly one implementation" asks for at the level of
representation as well as logic.

No schema file changes. `docs/specs/` gains the description of the key, because
the change is normative: a reader is now entitled to rely on it.

## Consequences

- An auditor can distinguish "N unrelated dispatches of this step" from "one
  operator decision each, in a chain" — which is the question the operator's
  original complaint was really about, underneath the volume.
- The rendering improvement #24 could not make becomes possible, but is not
  mandated here. This ADR records the fact; whether the compact view folds it,
  and how it labels the fold, is a rendering decision that must not be smuggled
  in as a data-model change.
- `retried_trace_ids` is a set, not a single id, because a step may have
  accumulated more than one failed trace before the operator retried it. Writing
  only "the latest" would be a choice the eligibility gate does not make, and
  the point of this ADR is to record the gate's own decision rather than a
  neater version of it.
- A trace written before this change has no `retried_trace_ids` key. That is not
  a compatibility shim: absence means the tool did not record the fact, which is
  the truthful reading, and nothing may treat absence as "not a retry".

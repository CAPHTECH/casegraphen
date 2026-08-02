# ADR 0016: A Halt Is A Recorded Reason

## Status

Accepted on 2026-08-02. Refines ADR 0004 decision 2 and depends on the typed
refusal boundary (issue #22). Closes issue #23.

## Context

`run --step` advances one item; `run --frontier` advances one round (ADR 0004).
Neither loops, and ADR 0004 decision 2 says so deliberately: *"A round is still
one invocation, not a scheduler. It does not loop, wait for new work, retry on
its own, or own liveness."*

That decision was aimed at liveness ownership — daemons, message buses, retry
engines, the things ADR 0002 keeps out. A bounded loop over rounds that stops at
the first thing needing a different actor owns none of them: it never waits,
never retries on its own, and terminates by construction.

What blocked such a loop was not the ADR. It is that no caller can tell *why* a
round stopped without reading prose, because the halt vocabulary is spread across
four places:

- `run` statuses — `no_dispatchable_step`, `round_executed`, `step_executed`,
  `step_failed`, `transition_not_authorized`, plus `dispatch_in_progress` and
  `retry_required` carried as obstruction reasons.
- `packet apply`'s `paused_for_review`.
- Tool refusals — prose on stderr with exit 1 (#22).
- `readiness.waiting_cell_ids`, buried inside the readiness payload.

Two operator wishes look opposed and are not: work should keep advancing without
someone pushing each step, and a work item with insufficient evidence must be
left honestly unfinished. Both fall out of one invariant:

> **The ledger stops only for a reason it recorded.**

Satisfy that and advancing is automatic (no recorded reason, no stop) while
waiting is preserved (a recorded reason always stops).

## Decision

1. **One halt vocabulary, one implementation.** A typed halt reason is derived in
   one place and reported by every command that can stop. It is derived from the
   evaluation and the log — not recorded, and not re-decided per command. `run`'s
   existing statuses and the packet pause are expressed *in terms of* this
   vocabulary rather than beside it.

   | halt | What it needs | Who supplies it |
   |---|---|---|
   | `nothing_eligible` | nothing right now | — |
   | `dispatch_in_progress` | another process's started dispatch to finish, or an explicit `--supersede-trace` assertion that it is dead (ADR 0014) | outside this invocation |
   | `round_budget_exhausted` | another invocation | the operator |
   | `needs_review` | an accepted review of a named target | **a different actor**, holding `review` |
   | `needs_evidence` | evidence satisfying a named requirement | any actor holding `evidence-attach` |
   | `needs_external` | an external event a `waits_for` names | outside the system |
   | `needs_retry_decision` | a decision to retry a failed step | an operator, explicitly |
   | `needs_plan_review` | a transition class the accepted plan does not authorize | a plan reviewer |

2. **A halt is a resumable object.** Every halt carries the reason, the revision
   it completed through, the named target, and the operations that would clear
   it — as structured values, never assembled command strings (the rule #19
   landed for `next_operations` after a packet-controlled id was found injectable
   into one). This generalises `packet apply`'s pause; the packet path becomes one
   producer of the shared shape.

3. **`operate` loops rounds and halts on the first non-progress reason.** One
   invocation repeats the existing round selection until a halt other than
   "progress was made" is reached, then returns that halt object. It does not
   wait, does not retry, does not widen eligibility — the loop may only repeat the
   selection `run --frontier` already performs, never define its own notion of
   "safe work", because a second definition of eligibility is a decision rule in
   two places. `--max-rounds` bounds the invocation the way `--max-parallel` bounds
   concurrency.

4. **The actor seam is a halt, never a step.** `needs_review` is never something
   the loop resolves. A packet's pause (ADR 0015) is the same rule at packet
   granularity: one invocation carries one gate, so one actor, and an actor
   accepting its own claim is self-review wearing an independent check's log
   signature. No flag makes the loop pass a review.

5. **`waiting` is derived, not asserted.** The `needs_external` halt, and the
   `readiness.waiting_cell_ids` that feeds it, are the waiting state. Nothing
   needs a hand-written lifecycle transition to keep an unfinished item honest,
   and a `waiting` lifecycle remains available as an ordinary input fact for cases
   where a human wants to assert one anyway.

6. **One gate authorizes the loop.** A single `dispatch` gate covers the
   invocation, as it covers a round today (ADR 0004 decision 6); the store
   re-validates the recorded gate on every append regardless. `--max-rounds` is
   what keeps "one authorization" from meaning "unbounded work" — but it bounds
   **rounds, not steps**. A round dispatches up to `--max-parallel` steps, so the
   spawn bound an operator is actually authorizing is `max_rounds × max_parallel`.
   The formal model advances one step per round, so the two coincide there and the
   proof does not transfer; the report carries `steps_dispatched` alongside
   `rounds_used` so the quantity being bounded is visible rather than inferred.

## What the formal specification changed

The design was written as an FSL design-layer spec before it was implemented
(`docs/specs/operate-halt.fsl`, verified with `fslc`). Two things came out of it
that the issue's draft did not have.

**`dispatch_in_progress` was in scope and did not arrive in the first
implementation.** The Context above lists it as one of the four scattered
vocabularies this ADR exists to consolidate, yet the first `derive_halt` let a
step held by another process's started dispatch fall through to
`nothing_eligible` with no target and no next operation. That satisfies "the
ledger stops only for a reason it recorded" **vacuously**: `nothing_eligible` is
a word asserting there is no reason, used where there is one. It is now the
eighth member, and its clearing act is an assertion (ADR 0014), never a retry.

**`round_budget_exhausted` is a required member of the vocabulary, not an
implementation detail.** The draft bounded the invocation with `--max-rounds` and
listed six halt reasons, none of which covers "the loop stopped because its
budget ran out while work was still dispatchable". `fslc verify` reported exactly
that state as a reachable deadlock: work available, loop not enabled, no reason
named. A bound that stops the ledger without a recorded reason violates the very
invariant this ADR exists to establish, so the bound gets a halt reason like every
other stop.

**The deadlock check is the mechanical statement of the invariant.** Declaring the
spec's intended terminal states as `halt() != Progress` means any state where the
machine stops while the derivation still says `Progress` is reported as a
deadlock — which is precisely "stopped because nobody pushed". The property that
motivated the ADR is therefore checked, not asserted.

The spec also proves, unbounded under `fslc verify --engine induction`:

- **`INV-OPERATE-001`** — whenever the loop consumed a round, the only step whose
  state changed was a dispatchable one. This is decision 3 and ADR 0002's
  exclusions as one two-state rule: the loop never retries a failure, never waits
  out an external event, never authorizes a transition, and never accepts a review.
- **`INV-OPERATE-002`** — no actor accepts the claim it dispatched (decision 4).
- **`INV-OPERATE-003` / `INV-OPERATE-004`** — the loop is stopped if and only if
  the derivation names a halt, in both directions. The second direction is what
  stops the vocabulary from becoming a way to quit early.
- Every halt reason is reachable (so none is dead vocabulary) and the two seam
  halts have a proven exit (so no halt is a deadlock wearing a vocabulary word).

## Why not the alternatives

**Let the loop retry failed steps.** That is the retry engine ADR 0002 excluded
and ADR 0004 kept excluding. Retry stays an explicit act (`--retry-step`) and
appears as the `needs_retry_decision` halt.

**Let `operate` take a one-shot `--retry-step` consent.** Built and tested
first, and defensible: spent on the first attempt, it maps onto a single
operator retry decision followed by an ordinary round, which the formal model
already permits. Rejected because decision 3 says the loop "does not retry" and
"does not widen eligibility" without qualification, and a consent the loop
spends makes both claims need a footnote. `operate` now refuses `--retry-step`
outright, so its eligibility is identical to `run --frontier`'s with no
exception — an operator retries with `run --frontier --retry-step S`, then runs
`operate`. An invariant that can be stated flat is worth more than saving a
command.

The version before that was worse and is the reason this paragraph exists: the
flag was parsed once and re-applied every round, so a single consent became a
round-budget-bounded auto-retry — precisely the alternative above. The formal
spec did not catch it because it modelled retry as a single external act and
never modelled the flag at all; the proof was of a simpler machine than the one
built. See ASSUME-OPERATE-001 in `docs/specs/operate-halt.fsl`.

**Let the loop auto-accept reviews when the same actor holds the capability.**
This is ADR 0015's rejected alternative restated at loop granularity: a
"separately gated step" inside one invocation is separated by nothing the tool can
observe. The only boundary the tool can check is two invocations.

**Keep the vocabularies separate and have `operate` interpret them.** That is a
second statement of every halt condition living next to the first — the
one-rule-two-places shape every audit round in this repository has found a real
defect in.

**Let the loop stop silently at `--max-rounds`.** Rejected by the formalization,
above.

## Consequences

- #22 is a prerequisite: a halt that arrives as prose on stderr with exit 1 cannot
  be branched on, and tool failures must be distinguishable from halts.
- `run`'s report shape changes to carry the shared halt, which is a contract
  change under the `contract-change` skill.
- This is the execution surface: `adversarial-execution-reviewer`, with findings
  reproduced by hand.
- ADR 0002's positioning becomes demonstrable rather than asserted — a ledger that
  advances itself and stops legibly is the thing the "acceptance ledger under a
  runtime" claim describes.
- The tool still does not own liveness. Nothing waits, nothing polls, nothing runs
  unattended. `operate` returns.
- **The tool enforces that two different actor ids were used. It cannot enforce
  that two different minds were.** An agent registered as one actor that later
  reviews as another passes every gate. No design fixes this; it is the boundary
  of what a ledger can check, and `INV-OPERATE-002` is the strongest form of the
  check that exists.

## Open question this does not settle: handoff

A pipeline where step N needs step N−1's output still stops for a human, because
the execution-plan contract has no step-input field and ADR 0002 lists typed
handoff as an explicit non-goal.

Worth examining before assuming a new contract is needed: #18 made artifacts
content-addressed (`artifact:sha256-<hex>`) and reachable from a claim by
`derives_from`. A binding that can name an artifact id as input would give handoff
*through* the ledger rather than around it — auditable by construction, and no
data channel added. If that works it is strictly better than typed handoff; if it
does not, typed handoff is a contract-change proposal on its own merits, not a
hook bolted onto this one.

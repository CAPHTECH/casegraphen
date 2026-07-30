# ADR 0004: Advance The Whole Frontier In One Invocation

## Status

Accepted on 2026-07-30. Amends the first non-goal of
[ADR 0002](0002-graph-engineering-positioning.md), which declined a parallel
dispatcher.

## Context

`run --step` advances exactly one work item per invocation. For a case space
whose frontier holds ten independent items, ten invocations run ten workers
strictly one after another, so wall-clock is the sum of the workers rather than
the slowest of them. Fan-out over independent branches is the main thing the
graph-engineering discourse asks an executor for, and it is the capability this
tool most conspicuously lacked.

ADR 0002 declined it with this reasoning: *"Fan-out conflicts with the single
append-only revision chain; resolving that (batch morphism per round, or
optimistic append with retry) is its own ADR if we ever take it."*

**That reasoning was wrong, and this ADR exists mostly to correct it.** The
append-only chain constrains *appends*, not *execution*. It only conflicts with
fan-out if one insists that a round produce a single revision — a requirement
nothing actually imposes. Workers can run concurrently while their results are
appended one at a time, in a deterministic order, and the resulting log is
shaped exactly as if the steps had been run sequentially. The supposed conflict
was an artifact of assuming the answer had to be "batch morphism or optimistic
retry", and it survived because a measurement-shaped limit
(the O(n²) derivation, since fixed) made the whole area feel closed.

Two facts made the cost of a round cheap enough to be uninteresting: readiness
derivation is now linear (0.20 s at 10,000 cells, down from 32 s), so
re-deriving between applications is effectively free; and run directories were
already reserved atomically with `fs::create_dir`, so concurrent dispatch had a
safe rendezvous point from the start.

## Decision

1. **`run --frontier` executes concurrently and appends serially.** One
   invocation selects every plan step eligible under the same rules `run --step`
   uses, runs their workers concurrently, then applies the results one at a
   time in plan-step order. Each step keeps exactly what it has today: its own
   run directory and trace, its own evidence-attach morphism, its own transition
   morphism when authorized, and its own anchored trace hash. The morphism log
   is therefore indistinguishable from the log that N sequential `run --step`
   invocations would have produced, and replay, rebuild, and validate are
   untouched.

2. **A round is still one invocation, not a scheduler.** `run --frontier`
   advances the frontier once and returns. It does not loop, wait for new work,
   retry on its own, or own liveness. ADR 0002's rejection of daemons, message
   buses, and retry engines stands unchanged; what is amended is only the claim
   that concurrency itself was incompatible with the model.

3. **At most one step per work cell per round.** Two steps transitioning the
   same cell in one round would race semantically even though the appends are
   serial. A second step naming an already-selected work cell is not dispatched
   and is reported with a reason, exactly like any other ineligible step.

4. **Concurrency is bounded and explicit.** `--max-parallel <n>` caps how many
   workers run at once, defaulting to a small number. A wide frontier must not
   fork one process per item just because it can; the operator declares the
   budget.

5. **Application is deterministic and independently gated.** Results apply in
   plan-step order regardless of which worker finished first, so a replay of the
   same round produces the same log. Every per-step check that exists today runs
   at application time against the state as it stands at that moment: the
   step's declared success requirements, the plan's authorized transition
   classes, and the refusal to apply a transition that introduces a new hard
   obstruction. A step whose transition is refused for one of those reasons does
   not abort the round — its evidence is still attached and the remaining steps
   still apply, which is the same domain-finding treatment a failure gets today.

6. **The dispatch gate covers the round.** One `dispatch` operation gate is
   validated per invocation, naming one actor and the capabilities that cover
   every selected binding — the same shape `run --step` validates today, applied
   to a set instead of a singleton.

## Consequences

- Wall-clock for an independent frontier becomes the slowest worker plus the
  serial application, instead of the sum of the workers.
- Because applications are serial and each re-derives, a step that was eligible
  at dispatch time can have its transition refused at application time by a
  sibling that landed first. That is correct and must be reported per step
  rather than silently dropped; a caller comparing "dispatched" against
  "transition applied" will see the difference.
- This touches the execution surface, so the working agreement applies: the
  `adversarial-execution-reviewer` pass is required and its findings must be
  reproduced before they are accepted.
- ADR 0002's non-goal list keeps every other entry. Typed handoff, cost
  enforcement, model-identity pinning, and memory of past misjudgments remain
  out of scope, and none of them is a prerequisite for this.

# ADR 0014: Superseding A Started Dispatch Is An Assertion, Not An Inference

## Status

Accepted on 2026-08-01. Resolves issue #16, which required this decision
before an implementation because the obvious fix trades one defect for
another.

## Context

A `started` execution trace blocks its step, so one accepted step has one
dispatch. `select_steps` made one exception: when `--retry-step` named the
step **and** the trace's `metadata.reserved_base_revision_id` differed from
the current revision, the trace was treated as superseded.

That condition is not about the dispatch. A revision moves on *any* append,
including a sibling step of the same plan finishing, so a live dispatch read
as dead the moment anything else was recorded. Reproduced with three ordinary
`run --step` invocations and no fault injection: two workers ran concurrently
for one accepted step, and both evidence appends were then refused on a stale
base revision, so the log did not record what had actually run twice.

Deleting the exception is one line and closes the race — measured, the
reproduction drops from two concurrent workers to one. It also removes the
only recovery from a **killed dispatcher**, which leaves a `started` trace
forever and blocks its step permanently with no CLI path back. That is a
denial of service in place of a race, not a fix; the existing test
`native_run_frontier_retry_recovers_stale_started_trace` documents exactly
that recovery.

So the question is not whether to keep the exception but what may trigger it.

## Decision

**The operator asserts which dispatch is dead, naming it, and the tool records
the assertion.** A new repeatable flag on `run`:

```
--supersede-trace <trace-id>
```

- It names a specific `started` trace, not a step. "The dispatch that trace
  records is no longer running" is a fact about a process on a host, which
  only the operator can know and this tool has no way to observe.
- It is refused unless the id resolves to a trace of this plan whose
  `dispatch_state` is `started`. An id that is unknown, already `failed`,
  already applied, or belongs to another plan or step is a refusal, not a
  no-op — the same shape as a stale `--base-revision-id`.
- If a *different* dispatch started for that step since the operator looked,
  its trace is not the one named, so it still blocks. The assertion cannot
  supersede a dispatch the operator never saw.
- The superseding trace records `metadata.superseded_trace_ids`. The gate
  already names the acting actor, so the assertion is attributable, and the
  trace is anchored (ADR 0013), so the record of it is covered by the trace
  content hash.

`--retry-step` returns to its stated meaning: retry a step whose previous
attempt **failed**. It no longer inspects `reserved_base_revision_id`, and no
combination of flags infers liveness from the graph's shape.

## Why not the alternatives

**Process identity in the started trace.** Recording the dispatching pid and
start time, and checking liveness at retry, is cheap on one host and wrong
everywhere else: a store on a shared filesystem, a dispatcher in a container
with its own pid namespace, or plain pid reuse — which this project already
records as a live concern in residual risk 4 for the process-group case. It
would replace a signal that is wrong in an obvious way with one that is wrong
in a subtle way.

**A lease with a deadline.** The binding carries `timeout_ms`, so a `started`
trace could expire after that plus a margin. It needs no assertion, but it
makes a clock a trust input: a suspended laptop or an overloaded host expires
a dispatch that is still running, and the failure is silent and timing
dependent. The whole point of the change is to stop inferring liveness from
something that is not liveness.

## Consequences

- `run` gains one flag. `docs/specs/casegraphen.md`, `src/cli_usage.txt`,
  `README.md`, and the operating skill carry it, and `tests/cli_surface.rs`
  enforces that they do.
- The recovery an operator performs is visible in the log as their claim,
  where before it was inferred from an unrelated revision comparison and
  recorded nowhere.
- A killed dispatcher still needs a human to notice and assert. That is the
  intended cost: the alternative is a tool that guesses, and this one is
  built on the premise that it does not.
- `native_run_frontier_retry_recovers_stale_started_trace` moves to the new
  flag; it was passing on a code path that could not tell a dead dispatcher
  from a live one, which is what made the defect invisible.

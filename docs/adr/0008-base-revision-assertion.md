# ADR 0008: A Base Revision Is An Assertion, So `current` Will Not Be Added

## Status

Accepted on 2026-08-01. Resolves issue #6, which asked for
`--base-revision-id current` or for following the latest revision automatically
when there is no conflict, and required this decision to be recorded before any
implementation.

## Context

Every mutating command takes `--base-revision-id`, and a morphism applies only
when that id matches the replayed current revision. A stale base revision is a
refusal, never a merge. The request was to let the tool read the value for
itself, either always (`current`) or when nothing "semantically conflicts".

The issue posed three questions in order, and the answers below are the
decision.

## Question 1: what does a base revision assert, and to whom?

It asserts **which state the caller decided against**, to two audiences:

- **To the tool at apply time.** The equality check is the only concurrency
  control a case space has. Between a caller reading state and issuing a
  command, another actor may have appended — `run --step` alone appends up to
  three entries — and the refusal is what turns that overlap into a visible
  event instead of a silent interleaving.
- **To every later reader of the log.** The morphism's `source_revision_id` is
  the durable record of what the actor saw when it proposed. "Who decided what
  against what" is readable from the log only because that field was asserted
  by the decider, not resolved by the tool.

So the answer to the issue's conditional — "if the tool relies on nothing here,
the refusal is theatre" — is that the tool and the log both rely on it. The
refusal stays.

`current` would make the assertion empty. A value the tool reads for itself
matches by construction on every invocation, including exactly the invocations
the check exists to catch: the ones where the graph changed between the
caller's decision and their command.

## Question 2: is there a derivable class of entry that cannot conflict?

No class survives inspection, for a structural reason: **the read-set of a
pending decision is not local to the ids it names.** Readiness, the frontier,
and blockers are derived from the whole graph on every command; evidence
coverage is derived from the log; gate validity depends on capability cells and
the source boundary. An appended entry that touches none of the ids in a
pending morphism can still change whether that morphism should have been
proposed at all — a new contradiction, a retired capability's scope, a coverage
pair going live.

Deriving "provably cannot conflict" would therefore mean implementing, next to
the revision check, a second statement of every decision rule's read-set. That
is a decision rule in two places, and every defect fixed in this repository's
audit rounds was exactly that shape. The class is not derivable at a cost this
codebase should pay, so automatic following is rejected with it.

## Question 3 and the resolution

With (2) empty, question 3 does not arise. The resolution the issue itself
anticipated is the one taken: **keep the refusal, and reduce the cost of
re-reading instead.**

- Mutating responses already carry `record.current_revision_id`, and the skill
  now teaches taking the next base revision from the response of the command
  that just wrote (issue #10), so the steady-state cost of the assertion is
  zero round-trips.
- The gate-profile work (issue #1) removes most of the surrounding repetition.

## Consequences

- `--base-revision-id` remains required and literal on every mutating command.
  No `current`, no alias, no environment fallback, no auto-follow. A future
  request for one should be answered by pointing here.
- The refusal message may say how to recover (re-read and re-decide), but the
  recovery is the caller's act, not the tool's.
- The skill documents the response-derived revision as the normal flow and
  `space inspect` as recovery, which keeps the assertion cheap enough that
  convenience pressure on it stays low.

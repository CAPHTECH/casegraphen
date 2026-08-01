# Issue sweep — 2026-08-01

Date: 2026-08-01
Scope: the fifteen issues open at `79e0d24`, worked as one pass. Implementation
was delegated; every behavioural claim below was re-executed by hand against a
real store before it was accepted, per the working agreements.

## What changed, and why it is what it is

**Two defects, both from the fourth adversarial round.**

*A cleanly exiting worker could leave descendants while the record denied it*
(#7). The clean-exit branch returned `descendants_may_survive: false` as a
literal and never signalled the process group. One shared rule now runs on all
four exit paths. Two corrections followed, each from a later attack that was
reproduced first: a `kill` that could not be spawned was being read as a group
that was empty, and the launcher was resolved by first match over a list
including `/opt/homebrew/bin`, measured group-writable on a normal developer
machine — the same population `--enable-worker shell` admits. The process
actually spawned is `setsid <pinned command>`, so that path let an approved
worker choose what every later dispatch executed while the binding, plan, and
acceptance hashes all still verified. System paths only now.

*The worker report and both raw streams were not anchored* (#8), while §2.6
described them as a link in the audit chain. The trace is `v2` and folds in
their hashes before it is hashed and anchored, with verification at the one
site that already verifies the trace. Confirmed load-bearing by disabling the
verification and watching three of the four tamper tests fail.

**One defect nobody had filed** turned up while settling #13: a genesis
evidence cell could name an id nothing had created, and the cell later created
with that id was born covered by trusted evidence with no review naming it.
A control run isolated the coverage claim as the only difference. Genesis
coverage now counts only for targets genesis itself materialized.

**Four decisions recorded rather than implemented.** `--base-revision-id
current` is refused, with the reasoning (ADR 0008): the assertion is what makes
concurrent use safe, and a tool-resolved value matches by construction on
exactly the invocations the check exists to catch. The guided CLI walkthrough
(#5) is declined because the guidance already ships where it belongs. The
log-tail anchoring duty (#15) gets a worked recipe rather than a read-side
assertion flag, because a refusal only some read commands honoured would be
this project's recurring defect shape. `space replay` stops emitting the log
twice but keeps the genesis payload mirror (ADR 0011), because a report's
`morphism_log` must be the stored one.

**Two dependencies, both measured before the decision.** `arbtest` at two
crates (ADR 0009) and `serde_path_to_error` at one crate beside the serde stack
already present (ADR 0010). ADR 0006 requires the `cargo tree` output for that
proposal rather than precedent, and both records carry it.

**Two rules specified in FSL before implementation and proved by induction**:
gate-profile resolution and the strict exit mapping. In both cases the proved
invariants became the acceptance criteria, and in one case that mattered:
close-check's first draft derived the exit code from a narrower
hard-obstructions rule that disagreed with the `closeable` verdict the command
already computed — one question answered twice, caught because the spec said
what the answer had to be.

## The shape that keeps recurring

Four earlier rounds concluded that the defect here is always **a rule that
exists in one place and not in its sibling**. This pass produced three more
instances and one near miss, which is worth recording because none of them was
a missing idea:

- the exit-code rule derived separately from the verdict it was meant to report
  (caught in review, before shipping);
- the containment rule made conditional on one path and not its siblings — my
  own correction, reintroducing the asymmetry the original fix removed;
- coverage read from a claim whose target the trust root never created, while
  every other coverage path required the target to exist;
- a launcher resolved by a rule that the thing it wraps is not resolved by.

The mitigation that worked was not vigilance. It was writing the rule down
where both siblings had to read it — one shared function, or a spec the
implementation had to refine.

## What was measured, not assumed

| claim | measurement |
|---|---|
| `--output` suppresses stdout | 0 bytes, on `lift native` |
| `space replay` duplication | 53,750 → 26,878 bytes on the 9-cell example |
| `space inspect` growth | ~439 B per revision, the reason `cur()` moved to recovery |
| `arbtest` tree | 2 crates (proptest 28, quickcheck 15) |
| `serde_path_to_error` tree | +1 crate over serde + serde_json |
| candidate launcher directories | `/opt/homebrew/bin` writable, `/usr/bin` and `/bin` not |
| batch attach atomicity | current revision unmoved after a refused second input |

## The fifth round, run against this pass's own output

Three adversarial rounds were run against the work above — the worker
containment fix, the gate profiles, and batch evidence attach — plus a fourth
against the lift adapters, the area issue #14 had recorded as never attacked.
Every finding below was reproduced by hand before it was accepted.

**The writer was still enforcing a subset of the loader's contract.** Round
four closed this class by having `append_morphism` validate the resulting
state, but against `validate_materialized_log`, whose reference check covers
relation endpoints and projection lists only. The loader also runs
`validate_native_case_space`, and the import path already called it — with a
comment explaining exactly why. Three ordinary gated commands reached the gap:
an `evidence attach` whose cell carries a mismatched `space_id` or a blank
title, and a `morphism apply` retiring any relation. Each wrote successfully,
each then failed every derived command permanently, and `space validate`
reported `valid: true` while `space rebuild` reported success — the two
commands the policy names as audit and recovery. The fix is the call the
import path makes.

**A coverage claim could name a work cell**, and the evaluator reads coverage
against a work cell as satisfying every evidence and proof requirement that
cell has. One attach plus one review cleared a blocking requirement that no
morphism named and no reviewer saw. Two reviewers found this independently.
`run --step` had already restricted its own coverage targets to evidence
cells; `evidence attach` asked only whether the id existed. One rule now.

**An imported workflow graph could declare its own trust**, in two ways, while
ADR 0003 stated that none survived. One of them was the sibling of a rule
enforced in the same function: `accepted_evidence` was refused for exactly
this reason while `source_backed_evidence` reached the same outcome under a
different label.

**`--retry-step` can supersede a live dispatch** — filed as issue #16 rather
than fixed. Revision staleness is being used as a liveness signal, and a
revision moves on any append, so a sibling step finishing makes a running
dispatch read as dead: three ordinary `run --step` invocations produce two
concurrent workers for one accepted step, reproduced here. Deleting the
escape is one line and closes the race, but the escape is what recovers a
killed dispatcher, so removing it trades a race for a denial of service. What
may supersede a `started` trace is a decision, and this pass has already shown
what improvising one costs.

That last point is the honest summary of the round: of five findings, four
were the same shape as everything else in this document, and the fifth is
being left to a decision rather than a fix at the end of a long session.

## The declared-unattacked areas, driven

Issue #14 recorded four areas no adversarial round had ever attacked, on the
premise that an unattacked area is not an area that held. It was right twice.

- **The lift adapters** gave up two trust values, above.
- **`--retry-step`** gave up the live-dispatch race, above.
- **Reader-thread and capture paths held.** The question that matters since the
  trace started anchoring both raw streams is whether a recorded hash can
  disagree with the file on disk — that would be a store that bricks itself on
  its own next read. Driven with a worker that holds stderr open past the
  reader grace and exits clean: all three hashes matched and a later read
  verified them. `CaptureProgress::record` updates the file and the hasher in
  one critical section and `seal_capture` stops both together, so the two
  cannot describe different prefixes.
- **A dispatcher killed mid-dispatch leaves a coherent record.** Driven: the
  trace stays `started`, the store stays valid, a sibling step still runs, and
  the killed step stays blocked — which is what `--supersede-trace` now exists
  to release.
- **Non-Unix was overclaimed rather than broken.** The crate builds for
  `wasm32-unknown-unknown`, but §2.3 said an execute-bit check runs "on every
  host" and it does not run off Unix at all. Now residual risk 9.

What is still not driven — the case-lock stale-break race, the
canonicalize-to-spawn TOCTOU, and the three containment limits that need a host
with `setsid` — is recorded in the residual risks, not in an open issue. A
permanently open "nobody has attacked this" ticket makes it easy to read the
absence of news as good news, which is the mistake it was filed to prevent.

## Where the next round should start

Every issue open at `79e0d24` is closed, and so is #16, which this pass
raised and then answered. Nothing is being tracked in an open ticket, which
means the next round starts from two places rather than from a list:

- **The residual risks** in `docs/security/worker-execution-policy.md`. Three
  of the nine were written or rewritten today, and risk 9 is new: worker
  execution is a Unix control surface and no non-Unix host has been driven.
- **This document**, for the shape. Four earlier rounds and this one all
  reduce to the same failure, and the four instances above were found by
  looking for siblings — a rule enforced here and not there — rather than by
  reading code for bugs.

One coverage gap is worth repeating because no amount of review closes it:
this host has no `setsid`, so the two real-binary containment tests take the
"no utilities" branch and cannot fail here no matter how broken the code is.
The property tests carry the rule; the platform coverage does not exist. A
Linux CI job running those two tests would settle it, and would also let
someone drive the three containment limits in residual risk 4, which are
code-read only.

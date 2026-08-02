# ADR 0017: A Held Lock Is Never Inferred Dead

## Status

Accepted on 2026-08-02. Resolves issue #30.

## Context

`CaseLockGuard::acquire` (`src/native_store/support.rs`) serializes every
durable write to one case space. Nothing else does: the morphism log is
append-only and hash-chained, so two concurrent appenders is not a lost update,
it is a broken chain.

The shipped implementation decided a lock was abandoned from **file mtime age
alone** — `age >= LOCK_STALE_AFTER` (60 s) — and then called
`remove_lock_if_owned(&path, &observed_token)` and proceeded. Two facts make
that inference unsound rather than merely optimistic:

- **Nothing refreshes the lock file after `create_new`.** There is no heartbeat
  while a lock is held, so the file's age measures *how long the holder has been
  working*, not how long it has been absent.
- **The pid in the ownership token is never checked.** The token carries
  `pid={}`, and no code in `src/native_store/` asks whether that process is
  alive.

So `LOCK_STALE_AFTER` was doing two jobs that need different evidence:
reclaiming a lock from a **crashed** process, and not stealing one from a
**slow** process. It answered both with elapsed time, which distinguishes
neither.

The compare-and-delete that follows does not repair this. It proves the lock has
not been *replaced* since it was observed — a condition that holds precisely
when the holder is alive and still working on the same operation.

`LOCK_WAIT_BUDGET`'s own comment already records the number that makes this
reachable: one gated `cell transition` on a 4,000-cell space takes 3.0 s. Sixty
seconds is one contended order of magnitude away, and the comment's claim that
the budget is "kept well under `LOCK_STALE_AFTER` so a waiter gives up before it
could mistake a live holder for an abandoned one" is only true of a waiter that
arrived at the same time as the holder. A waiter that arrives 45 s into a slow
write is 15 s from breaking it.

`docs/specs/case-lock.fsl` formalizes the acquire protocol and `fslc verify`
produced the counterexample in four steps, with no dead process anywhere in the
trace:

```
step 1  time_passes()        aged: false -> true
step 2  acquire_free(2)      lock: none -> 2, st[2]: Idle -> Holding
step 3  observe_lock(0)      observed[0]: none -> 2
step 4  steal_stale_lock(0)  lock: 2 -> 0, st[0]: Idle -> Holding
                             => two simultaneous holders
```

The second half is what makes it worse than a race: **the displaced holder is
never told.** On release, `remove_lock_if_owned` finds a token that is no longer
its own, returns `Ok(false)`, and the operation reports success. `space validate`
would catch the damaged chain afterwards; nothing catches it at the time, and
nothing tells the process that lost.

Issue #32 records an intermittent `store_integrity` failure under full-suite
parallelism that matches this mechanism. It is **not** a reproduction — 60
surviving case spaces from a failing run all validated clean — and this decision
does not rest on it.

## Decision

**The tool never infers that another process is dead.** The mtime-age staleness
check and `LOCK_STALE_AFTER` are deleted. A waiter that cannot acquire the lock
within `LOCK_WAIT_BUDGET` refuses with `LockUnavailable`, naming the lock file's
path, and leaves the file exactly as it found it.

This is the stance ADR 0014 already takes one layer up. There, a `started`
execution trace blocks its step, and the recovery is not an inference from
elapsed time but `--supersede-trace <trace-id>`: the operator asserts, after
externally establishing it, that a particular dispatch is dead. Breaking a lock
on age is the same inference ADR 0014 refused, made silently and with a worse
consequence.

**Lock recovery becomes an operator act.** The refusal names the file; removing
it is the assertion that its holder is gone. No CLI command is added for this —
the act is `rm` on a path the refusal already printed, and a command would
mostly serve to make the assertion feel like a tool decision.

### What was rejected

- **Check the pid.** `kill(pid, 0)` is the closest thing to asking the actual
  question, and the pid is already in the token. It needs `libc`: `std` exposes
  no liveness probe, and `unsafe_code` is forbidden by lint here. That is a new
  dependency, which under ADR 0006 must be argued with its own measured
  `cargo tree` — for a check that still cannot see across a container or host
  boundary, and that pid reuse makes probabilistic. Not worth it to preserve an
  automatic recovery this crate does not otherwise offer.
- **Heartbeat the mtime.** It makes age mean what the check assumed, and it puts
  a background refresh inside a store operation — a scheduler-shaped thing in a
  crate whose positioning ADR (0002) excludes schedulers, for the same recovery.

Both were rejected for the same underlying reason: they buy back an inference
the design is better off not making.

## Amendment, 2026-08-02: the recovery act needed a guard on the write

An adversarial review of this decision found, and reproduced, that making `rm`
the documented recovery act shipped an integrity defect. Nothing between
acquiring the guard and performing the durable append re-checked that the
process still held the lock, so one well-timed `rm` — the act this ADR
prescribes — put two writers into one case space:

```
-- lock observed ; operator performs ONE rm
A exit=0        A stderr: (empty)
B exit=0        B stderr: (empty)
log lines: 1 -> 3, sequence 2 present twice with the same entry_id
both reports claim revision:cell-transition:work~3areview-native-contract:2
space validate / rebuild / history / inspect  ->  exit=1 store_integrity
```

Two silent successes and a store no command in the tool can read again, with no
in-tool repair. Reproduced independently before being accepted.

The mistake was not in the decision but in how its formal statement was scoped.
`docs/specs/case-lock.fsl` gated `INV-LOCK-002` — "no process reports success
after losing its lock" — on `not operator_intervened`, and that gate was
described in this ADR as the honest part, on the grounds that a human who
removes a lock file can always break things. That reasoning is wrong in one
specific way: **this decision moved the operator's removal onto the normal
recovery path.** An exemption that once covered an exotic act now covered the
ordinary one. An invariant excused on the path the design tells people to take
is not an invariant.

**`CaseLockGuard::still_owned` is added, and every durable write calls it
immediately beforehand.** It re-reads the lock file and compares against the
token this guard was given, refusing with `LockUnavailable` when it has moved.
This is not a liveness inference and does not reopen what this ADR closed — it
is the tool establishing that *it* still holds what it acquired, which is the
one question it can answer about itself. It converts silent destruction into a
refusal before any byte is written.

`INV-LOCK-002` now holds unconditionally, and `INV-LOCK-005`/`INV-LOCK-006` were
added: a write happens only in a step where the writer held the lock, and a
displaced holder never writes. All proved by k-induction.

**The residual, stated rather than closed:** the check is not atomic with the
append. A TOCTOU window remains; it shrinks from the whole operation — 3.0 s
measured on a 4,000-cell space — to microseconds. The spec records this as
`ASSUME-LOCK-001`, and no comment in the code may claim the window is gone.

**Which writes are guarded.** Naming the durable writes turned out to be the
part worth doing carefully: the obvious two are `append_verified_log_entry` and
`write_new_case_space`, but `rebuild_case_space_inner` also writes — a missing
periodic snapshot, and the log head. The head write is the sharper case, because
on the `repair_lagging_head` path it *overwrites* the head with a `latest`
computed from a log read taken under a lock the process may no longer hold. A
displaced rebuild racing an append could therefore write a head naming an
earlier entry than the log contains: either the crash-shaped "head lags the log"
state manufactured out of nothing, or a head rollback, which residual risk 2 of
`docs/security/worker-execution-policy.md` names as the thing this store must
not produce. All of them are guarded.

**Binary-level coverage was declined, not overlooked.** The displaced-writer
test drives the real `CaseLockGuard` and the real append in the real call-site
sequence, but at unit level. An integration test through the binary would have
to widen the lock's hold window with a large case space and then race an
external `rm` against it, which makes its pass depend on wall-clock timing on a
loaded machine. Issue #32 exists because this suite already has tests with
wall-clock assumptions, and its rule is that a test which depends on timing it
cannot control should have what it asserts changed rather than be made to retry.
Adding a new one to cover this would contradict that on the same day it was
written down.

## Consequences

- A crashed holder strands its lock until a human removes the file. This is a
  real regression in unattended operation and is recorded as such, not
  minimized: `MODEL-LOCK-007` in `docs/specs/case-lock.fsl` makes it a reachable
  state of the model rather than a footnote. The trade taken is that a stranded
  lock is loud, bounded, and recoverable, while a broken chain is silent and
  is the thing every other guarantee in this crate rests on.
- **A latent orphan path became permanent, and was fixed as part of this
  change.** An adversarial review of this decision found that
  `CaseLockGuard::acquire`'s read-back arm returned `Io` on a non-`NotFound`
  read failure without removing the lock file it had just created — no guard is
  constructed on that path, so no `Drop` cleans it up either, while the
  structurally identical `write_all`/`flush` arm three lines above does remove
  its own file. Before this ADR the asymmetry was survivable, because the
  staleness check reclaimed the orphan after 60 s. Removing that check makes it
  permanent, so the decision here turned a latent inconsistency into a real one
  and had to fix it. The fix routes through `remove_lock_if_owned` rather than
  `fs::remove_file`: the read that just failed *is* the check for "did someone
  replace this file", so the branch cannot know the file is still its own, and
  a compare-and-delete is the only removal this ADR permits.

  Stated as the invariant it belongs to: **the tool never leaves behind a lock
  file it created and does not hold, and never removes one it did not create.**
  The four sibling paths are the write failure (removes its own), the read-back
  failure (now removes its own), the read-back "ownership changed" case (must
  *not* remove — the file is someone else's, and returning `LockUnavailable`
  there is correct), and `Drop` (removes if owned). All four now answer through
  the same compare-and-delete.

  Not reproduced, and recorded as such: reaching that branch needs a
  non-`NotFound` read error on a file this process has just written
  successfully, which cannot be produced without OS-level fault injection or a
  seam in production code added solely to reach it. The fix rests on the
  code-shape argument and on reusing an already-tested helper, not on a
  reproduction.
- `LOCK_WAIT_BUDGET`'s comment loses its reference to `LOCK_STALE_AFTER`. The
  budget now bounds patience only, and no longer needs to be "kept well under"
  anything.
- `docs/specs/case-lock.fsl` proves, by k-induction rather than bounded model
  checking, that no process is displaced except by the operator's own act
  (`MODEL-LOCK-009`) and that mutual exclusion follows from it (`INV-LOCK-001`).
  Both are conditioned on `not operator_intervened`, deliberately: with an
  operator able to remove any lock file, unconditional mutual exclusion is not
  provable, and claiming it would be a spec proving something the design does
  not deliver. What is provable, and what this ADR promises, is that *the tool*
  never breaks a live lock.
- Residual risk 8 of `docs/security/worker-execution-policy.md` (adversarial
  lock denial) is unchanged in kind and slightly worse in degree: an attacker who
  can create the lock file can now hold a case space until a human intervenes.
  It was already able to re-create the file faster than any waiter's budget; the
  delta is that waiting no longer eventually wins.

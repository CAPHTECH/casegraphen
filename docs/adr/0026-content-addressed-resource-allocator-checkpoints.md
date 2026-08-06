# ADR 0026: Content-addressed resource allocator checkpoints and safe compaction

- Status: Accepted for experimental v0
- Date: 2026-08-05

## Context

The operational allocator journal is the authority for exclusive resources,
capacity, dispositions, idempotency, and reviewed deployment bindings. Replaying
the complete append-only journal on every operation preserves those decisions
but makes startup and append latency grow with journal length.

## Decision

The allocator may publish a strict `resource.allocator_checkpoint.v0` derived
only by complete replay. A checkpoint binds the allocator instance and journal
location, allocator configuration, exact covered sequence and terminal hash,
prefix hash, and replay-derived state. Its filename and payload are
content-addressed. A checkpoint is an accelerator, never a replacement for
journal authority.

Normal replay validates the newest checkpoint and replays the active suffix.
Independent verification replays active and archived event bytes from sequence
one before producing an opaque checkpoint proof. Only that proof plus an
explicit `resource.allocator_retention_policy.v0` permits compaction.

A long-lived allocator retains the last canonically replayed `ReplayState` in
process and mutates it only after the event file and directory are durable.
Every mutation takes an advisory lock shared by allocator processes. A small
`.allocator-head-hint` invalidates another process's cache; it is not authority.
Missing or malformed hints, a hint/cache mismatch, or the presence of the next
active/archive event forces canonical replay. Event create-new publication is
still the concurrency backstop for an older writer that does not honor the
lock.

The in-process replay state is ephemeral authority for a hot allocator
session. It is valid only while the journal directory remains under one
private service identity and all supported mutation goes through canonical
allocator writers. Identity bytes and checkpoint/compaction inventories are
fingerprinted; changes invalidate the cache. Head mismatch and a newly
published next sequence also invalidate it. The hot path intentionally does
not re-hash every historical event byte because doing so would recreate full
replay on every operation. An unsupported in-place mutation of an older event
therefore does not change an already-derived hot decision; restart, explicit
full replay, checkpoint verification, and audit recovery re-read the chain and
refuse the mutation. Shared writable journal directories are outside this v0
trust boundary.

The advisory lock has a bounded wait and returns a typed `WriterBusy` refusal;
process crash releases the operating-system lock. The cached state maintains
derived historical-identity, active identity, resource-holder, and rate-group
occupancy indexes. Both the compatibility slice evaluator and allocator index
path delegate to one canonical conflict/capacity decision function. Reserve,
disposition, reviewed-authority lookup, and replay therefore avoid scanning
the full active or historical collections. Append outcomes may return a
bounded operation snapshot; callers that
need the complete historical disposition projection request `snapshot()` or
full replay explicitly. Neither index nor the head hint is serialized as
allocator authority.

Head-hint publication happens after the authoritative event is durable. A
hint failure is reported as `head_hint_healthy: false` in the bounded operation
snapshot but cannot turn the committed allocator operation into an error.

### Versioned API migration

The existing `reserve`/`disposition` methods and
`ResourceAllocatorOutcome.snapshot` keep the 0.8 complete
`ResourceAllocatorSnapshot` contract. Long-lived hosts and fleet loops opt in
to `reserve_bounded`/`disposition_bounded` (and reviewed equivalents), which
return `ResourceAllocatorOperationOutcome` with generation, terminal hash,
active/disposition counts, allocator-configuration hash/capacity count, and
hint health. This avoids silently
changing the public Rust type while making the O(1)-sized response explicit.
Callers can migrate first to the bounded methods and request `snapshot()` or
`full_replay_snapshot()` only when they need complete inventory. The
experimental MCP response returns `active_reservation_count`; this wire-level
change is recorded as an experimental-v0 migration.

Compaction hard-links covered event bytes into the archive, syncs them,
publishes a content-addressed `resource.allocator_compaction.v0` record, and
only then removes active duplicates. The archive remains part of full replay.
Crashes may therefore leave duplicate active/archive bytes or an archived-only
prefix; disagreement, gaps, substitution, configuration mismatch, journal
relocation, ambiguous checkpoints, and published partial JSON all fail closed.
Pending temporary files do not affect replay.

The operational host can enable this lifecycle with a retention policy and a
positive event interval. It reports checkpoint and compaction hashes in the
operation response. Supplying only one maintenance option is invalid.

## Consequences

- Restart replay uses the newest checkpoint plus suffix; ordinary operations
  use a validated in-process state and do not replay or clone full history.
- Full replay and audit recovery remain available and authoritative.
- Checkpoint creation and verification deliberately retain an O(events) path.
- Cross-process writer serialization adds one advisory-lock operation per
  mutation. Contention is bounded and typed; a crashed process releases the OS
  lock. Stale hint bytes cause replay rather than recovery refusal.
- Archive growth is not deletion; an external, separately reviewed retention
  policy would be required before authoritative event bytes could be removed.
- Copying a journal to another path requires an explicit future migration
  protocol rather than silently reusing its checkpoint authority.

## Promotion evidence

Experimental promotion requires retained 512, 10k, and 100k event reports with
append latency distribution, restart/suffix/full replay latency, checkpoint
creation/verification latency, checkpoint size, compaction latency, and peak
memory. CI runs bounded correctness and 512-event pilots; 10k and 100k are
release evidence lanes. Any authority-semantic mismatch, replay divergence, or
unbounded regression blocks promotion regardless of latency.

The retained 10k and 100k reports from clean exact revision
`9b23383463cb1f1fafb666e7fb87a596b3e090e2` pass these configured resource
budgets, including 10,000 shared-read all-active reservations in each lane.
They remain unattested release-candidate observations with
`promotion_authority: false`; passing allocator scale budgets does not itself
promote the experimental contract.

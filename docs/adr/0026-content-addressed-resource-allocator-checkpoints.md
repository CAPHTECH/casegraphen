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

- Restart and ordinary allocation replay only the checkpoint suffix.
- Full replay and audit recovery remain available and authoritative.
- Checkpoint creation and verification deliberately retain an O(events) path.
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

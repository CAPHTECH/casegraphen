# ADR 0005: Snapshot Periodically, Not Per Revision

## Status

Accepted on 2026-07-31. Addresses candidate 1 of
`docs/audit/local-optima-audit-2026-07-31.md`.

## Context

Every `append_morphism` writes a complete snapshot of the case space for the new
revision. That was locally right and is still locally right on the read path:
`space replay` opens one file, verifies its checksum and its embedded log
prefix, and is done.

The cost sat behind a bigger one until yesterday. Readiness derivation was
O(n²), so the snapshot write was never the thing anyone noticed. Making
derivation linear (ADR 0002's amended context) promoted the snapshot to the
dominant cost, and the audit measured the inversion on a 10,000-cell space:

| Operation | Cost |
|---|---|
| `space frontier` (replay + derive) | 0.20 s |
| `space replay` (read + verify snapshot) | 0.29 s |
| `space validate` (**fold the whole log from empty**) | 0.42 s |
| **one append** | **1.02 s** |
| **disk added by changing one cell's lifecycle** | **+24 MB** |

The striking number is the third: folding the entire log from empty is less than
half the cost of a single append. The fold path is efficient; the snapshot write
is what is expensive.

`run --frontier` makes this the binding constraint on the feature shipped one
commit earlier. Each executed step appends three morphisms (evidence,
transition, trace anchor), so a four-step round on a 10,000-cell space spends
roughly twelve seconds writing twelve snapshots to serialize about one second of
parallel worker execution.

The design already says snapshots are not the source of state. `CLAUDE.md`:
*"The append-only morphism log is reconstructive: genesis carries its
materialization, so `space rebuild` can fold the log from empty… Do not
reintroduce state that only exists in a snapshot."* The first implementation
incorrectly concluded that this made every property of the per-revision
snapshot disposable. It did not: the current snapshot's embedded log was also
an independent second witness for the log tail. Changing the cache policy
without replacing that witness weakened tamper detection.

## Decision

1. **Snapshot the genesis revision and thereafter every Kth revision by log
   sequence.** K is a named constant in the store, not a CLI flag or a config
   file: the right value depends on nothing the caller knows, and an unnecessary
   knob is a second thing to get wrong.

2. **`replay_current_case_space` folds forward from the nearest snapshot.** It
   loads the newest snapshot at or before the requested revision, verifies it as
   it does today, applies the remaining log entries, and verifies the resulting
   state against the *target entry's* recorded `replay_checksum`. When no
   snapshot exists at or before the revision it folds from empty, which always
   works because genesis carries its materialization.

3. **Verify once per replay, not once per folded entry.** Checksumming every
   intermediate state would make replay O(K × space) and trade one problem for
   another. The end state is what must match the hash-chained log, and it is
   what gets checked.

4. **The log tail has its own independent anchor.** Every import and append
   atomically replaces a constant-size head file next to the log. The head
   records the last entry's `target_revision_id`, full entry hash, and
   `replay_checksum`. Every history/read/replay path requires that head to exist
   and match the log tail; replay additionally requires the folded end state to
   match the anchored checksum. A missing, malformed, or stale head is a refusal,
   including for `space validate` and `space rebuild`.

   This mechanism repairs a false claim in the original decision. The original
   reasoning treated a per-revision snapshot as merely a state cache. For the
   newest revision it also carried a second copy of the log, so
   `require_embedded_log_matches_prefix` pinned fields that `case_space_checksum`
   intentionally blanks. Sparse snapshots removed that second witness from up
   to K−1 tail entries. A hash chain and a checksum stored inside the same
   editable log are not an independent anchor.

5. **Existing snapshots remain readable, but an existing store needs a trusted
   head before this implementation will open it.** A store with a snapshot at
   every revision is a superset of the interval, so nearest-snapshot lookup
   still finds an exact match and `space rebuild` may leave extra snapshots in
   place. The head is different: silently deriving a missing anchor from the log
   it is meant to witness would provide no integrity. The provisioning step is
   therefore the explicit operator trust assertion
   `space rebuild --adopt-existing-log`. It is accepted only when the head is
   missing. Before creating the head, rebuild validates the log structure and
   hash chain, folds the complete log from empty while verifying every
   revision's `replay_checksum`, and verifies every existing snapshot against
   the corresponding folded state and embedded log prefix. A failed check
   writes no head. An existing valid head is left unchanged, while an existing
   malformed or disagreeing head is refused and never replaced. The residual
   risk is that the log was already tampered before adoption; the flag records
   the human operator's assertion that the pre-existing log is trusted. All
   normal imports and appends write the head themselves.

6. **`space rebuild` recreates the snapshots the interval calls for**, not one
   per revision, and still refuses to overwrite a snapshot that disagrees with
   the fold.

## Consequences

- Disk cost per append drops from O(space) to O(space)/K. The 34-append,
  10,000-cell cycle occupied about 52 MiB instead of roughly 800 MB, retaining
  the approximately 15× saving.
- The first periodic-snapshot implementation was a latency regression. Its
  sequence 2, 16, 31, 32, and 33 appends measured 0.77 s, 1.61 s, 2.33 s,
  2.64 s, and 0.77 s; all 34 appends took 54.26 s, 56% slower than the old
  per-revision-snapshot cycle's 34.7 s. The original 0.77 s claim measured only
  fold depth one and reported the best case as though it were representative.
- After maintaining the replay ID and reducer indexes across the fold,
  validating only references touched by each entry, and streaming full-log
  revisions instead of retaining them, the same fresh-store sweep measured
  0.83 s, 0.94 s, 0.94 s, 1.20 s, and 0.77 s at sequences 2, 16, 31, 32, and
  33. The 34-append total was 32.02 s, 7.7% below the old 34.7 s cycle.
- Replay of a revision that has no snapshot costs one snapshot read plus up to
  K−1 payload applications plus one checksum. Payloads are small — a
  cell-transition payload is one cell. The replay path builds indexes once and
  updates them incrementally; rebuilding whole-space ID sets or cloning the
  case space per folded entry recreates the regression above.
- This is the storage fix only. An append still serializes the whole space to
  compute the entry's `replay_checksum`, so append stays O(space) in CPU. Making
  that incremental is a different and much more invasive decision, not taken
  here.
- The tail anchor adds one small atomic file replacement per append. A crash
  between log append and head replacement leaves a stale head and therefore a
  deliberately unavailable store, not a silently accepted tail. The verified
  append path rolls back ordinary write/read-back failures, but the two files
  are not a transactional filesystem primitive.
- Full validation at 64 entries on the 10,000-cell case peaked at about
  1,139 MiB RSS, down from the reviewed 4.48 GB. Memory is now independent of
  revision count for materialized states, although whole-state checksum
  serialization still sets a substantial O(space) base.
- This touches the store, which is execution surface under the working
  agreement, so the `adversarial-execution-reviewer` pass is required and its
  findings must be reproduced before acceptance.
- `README.md`'s description of `space replay` reading "the current snapshot" and
  the corresponding wording in `docs/specs/casegraphen-native-case-management.md`
  become inaccurate and are updated with this change.

## Amendment to ADR 0002

ADR 0002's statements that every revision writes a full snapshot and that the
per-revision snapshot cost is the active storage ceiling are amended by this
decision. Genesis and every 32nd sequence now receive snapshots, intervening
state is replayed from the nearest snapshot, and every log tail is pinned by the
constant-size head described above.

# ADR 0011: A Report About A Case Space Does Not Carry The Log Twice

## Status

Accepted on 2026-08-01. Resolves issue #11, which measured `space replay` at
53,750 bytes on a 9-cell, 4-relation space and asked for the two duplication
questions to be decided rather than trimmed.

## Context

The measurement, from the issue:

| component | bytes |
|---|---|
| total replay output | 53,750 |
| top-level `case_cells` | 4,423 |
| genesis payload mirror | 6,234 |
| `history` (the log again) | 22,540 |
| `genesis_case_space` shell | 806 |

Two different things produce it, and they have different answers.

## Decision 1: `result.replay.history` is dropped from the report

`NativeCaseSpaceReplay.history` and `case_space.morphism_log` are the same
entries, serialized twice into one payload. `history` is the largest single
component of the output and it answers a question another command already
answers: `space history` exists, is narrow, and returns exactly the log.

`history` stays on the Rust struct — the topology commands fold it
(`native_case_topology_with_history`) and `validate_case_space` reads it — and
stops being serialized. The field is internal state of the replay result, not
part of the report about it.

This is not a schema change under `schemas/casegraphen/`: the replay envelope
has no shipped schema file, only the `highergraphen.case.native_store.replay.v1`
id constant, and no fixture, example, or test reads `result.replay.history`.
It is a change to what `--format json` returns, so it is recorded here rather
than left to a diff.

## Decision 2: the genesis payload mirror stays

`morphism.metadata.payload.added_cells` in the genesis entry is a byte-identical
copy of the top-level cells, and it is load-bearing: the log is reconstructive,
`space rebuild` folds it from empty, and that copy is what makes it possible
(`lift native` derives it, which is why authors do not write it).

A report that echoes a case space echoes its log, and the log's first entry
contains that copy. Suppressing it in reports would mean the report's
`morphism_log` is not the stored `morphism_log` — a second shape for the same
record, differing only in a report context. That is the divergence this project
keeps paying for, and it would cost more than the 6 KB it saves. With
`history` gone and `--output` taught (issue #9), the remaining size is not a
context cost.

## Consequences

- `space replay --format json` no longer has `result.replay.history`. A caller
  that wanted the log calls `space history`; a caller that wanted the folded
  state has `result.replay.case_space`, whose `morphism_log` is the same
  entries.
- The measured payload drops by the `history` component — about 42% of the
  output at the measured scale, and a larger fraction as history grows, since
  `history` grows with the log while the cells do not.
- Reports that echo a case space keep the genesis mirror, deliberately, and
  `docs/adr/0005` and the reconstructive-log semantics are untouched.

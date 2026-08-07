# ADR 0035: No Mechanical Transcription From An Analysis Space Into A Genesis Draft, For Now

## Status

Accepted on 2026-08-07. Resolves the transcription-assist question raised in
issue #123.

## Context

ADR 0003 §4 drew a deliberate line: a lifted workflow graph is an analysis
space with no capability cells, and making that work executable means
authoring a native genesis with explicit capabilities — "an intentional
re-declaration of authority, not a flag." Issue #123 asked whether the
*unintentional* part of that step — retyping the cells and relations a lifted
space already derived into a fresh genesis draft the operator then completes
with capability cells and re-lifts — deserves mechanical assistance, and asked
that the question be decided and recorded either way rather than left open.

The two loops published alongside this ADR
([`docs/guides/entry-ladder.md`](../guides/entry-ladder.md)) make the actual
size of that step measurable instead of assumed. The minimal governed genesis
is 108 lines for one work cell and one capability cell; the transcribable part
of a real analysis space — `case_cells` and `case_relations`, stripped of
everything the lift already discards or marks untrusted — is a small, regular
subset of that. There is no evidence yet of an operator repeatedly performing
this transcription at a size where hand-typing it is the bottleneck: the
friction actually measured and reported (#123's issue comment) was schema
field names on the *first* document written from nothing, which the shipped
examples now remove directly, and the doubled authoring surface in the
walkthrough genesis, which this same round fixed by deleting the derived
`payload`/`genesis_case_space` copy. Neither measured friction is the
transcription step this ADR is about.

## Decision

**No transcription tool now.** Building one would mean designing and
maintaining a second surface that reads a lifted analysis space and writes a
native genesis draft — new code, a new output shape to keep in sync with
`native.case.space.schema.json` as it evolves, and a new place for the
single-source rules in `CLAUDE.md` to be checked against. `CLAUDE.md`'s own
order of evaluation — "do nothing" before "minimal implementation," "adding
new code is the last resort" — has nothing to weigh this against: no measured
case establishes that transcription, rather than the two frictions already
fixed, is what makes the re-declaration step expensive.

This is a decision under current evidence, not a permanent one. If a future
report measures operators performing this transcription repeatedly, at a size
where typing it is the friction — the same evidentiary bar issue #123 itself
set for the publication work in this round — building the tool becomes a
proposal with its own ADR, sized against that report the way this round was
sized against #123's issue comment.

## Consequences

- No new command, no new output contract, no new code path from an analysis
  space to a genesis draft.
- The re-declaration of authority ADR 0003 describes stays a human act for
  every case space, not only the ones small enough that nobody asked for
  help.
- A future proposal to build this tool starts from a measured report of
  repeated transcription cost, not from this ADR's absence of one.

# ADR 0010: Adopt `serde_path_to_error` So Strict-Parse Refusals Say Where In The Document

## Status

Accepted on 2026-08-01. Resolves the decision issue #12 recorded, applying
ADR 0006's criterion with the required measurement.

## Context

Every input this project owns is strict (`additionalProperties: false`, serde
`deny_unknown_fields`), deliberately. The refusal a caller gets today is
serde_json's, e.g. `unknown field 'id', expected 'number' at line 175 column
9` — a line and column into a machine-generated document, with no statement of
which object rejected the field. A team lifting a GitHub issue snapshot wrote
a normalization script to get past exactly this.

The original reporter's failure had a cheaper root cause (the mirrored `gh`
records were closed while the `issue` object was open; making the openness
rule uniform shipped separately). The general problem remains for every strict
input this project owns: the case space, the workflow graph, execution plans,
worker bindings.

The alternatives were a dependency or hand-written path tracking through every
`Deserialize` — more code in this repository to maintain and a second place
for the rule to live, which is this codebase's recorded recurring defect.

## Measurement

Per ADR 0006, measured with `cargo tree` on 2026-08-01, in a project already
depending on `serde` and `serde_json` (as this crate does):

- baseline `serde` + `serde_json`: 6 transitive crates
- with `serde_path_to_error` 0.1.20: 7 transitive crates

The delta is **one crate** — `serde_path_to_error` itself; its dependencies
(`itoa`, `serde_core`) are already in the tree via `serde_json`. It is by the
same author as serde and contains no build script.

## Decision

Adopt `serde_path_to_error`, pinned in `Cargo.lock`, and route the strict
parse of every input contract this project owns through it, so a refusal
reads:

```
issues[47].closed_by_pull_requests[0]: unknown field "id", expected "number"
```

- **What it replaced:** the hand-written alternative — threading a path
  tracker through every strict `Deserialize` in the crate — and the status
  quo, in which the refusal is the interface but does not say which object
  refused.
- **Risk balance:** one crate, no build script, replacing a class of consumer
  normalization scripts written against our refusals. The first clause of
  ADR 0006 is satisfied by the second half: a strict contract whose refusals
  cannot be located invites callers to loosen inputs by trial and error.
- **One implementation:** the path-wrapping lives in one shared parse helper
  that all strict input entry points call, not per call site.

Not taken: naming the JSON Schema `$defs` entry in the message (the
reporter's full proposal). serde has no knowledge of the schema file, and
mapping Rust types back to `$defs` names would be a hand-maintained table —
a second statement of the contract. The path plus serde's own
`unknown field … expected …` carries the "where" and the "what was allowed";
the "why it is closed" stays documented in `schemas/casegraphen/` and the
`contract-change` skill.

## Consequences

- The crate gains one runtime dependency. `cargo package` keeps proving the
  standalone build.
- Refusal texts change shape. Anything asserting on the old
  `at line N column M` form is updated — fixtures are updated to the stricter
  behaviour, per the working agreements.
- New strict inputs must use the shared helper; a raw `serde_json::from_str`
  on an owned contract is a review flag.

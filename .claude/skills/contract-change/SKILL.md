---
name: contract-change
description: Use when changing anything under schemas/casegraphen/ — adding a field, adding a record type, or changing a report shape. Walks the decision of whether the change needs a new schema id, then the full set of places that must move together so the contract, the code, and the fixtures cannot drift apart.
---

# Changing a wire contract

The schemas in `schemas/casegraphen/` are the product. They are strict:
`additionalProperties: false` everywhere, so any unknown field is rejected at
parse time. That strictness means a field addition is not a free action — decide
its shape before writing it.

## Step 1: decide where the data goes

Ask in this order and stop at the first that fits.

| Question | If yes |
|---|---|
| Is this data free-form annotation nobody parses structurally? | Put it in the record's existing `metadata` object. No schema change. |
| Is this data consumed by a reducer or evaluator, but the record already has a `metadata` escape hatch the reducer reads (e.g. `metadata.payload`, `metadata.operation_gate`)? | Extend that convention and document it. No schema change to the case-space contract. |
| Is this a genuinely new record type? | New schema file, new `$id`, new Rust constant, new example. |
| Is this a required field on an existing strict record? | **Breaking.** It needs a new schema id (`.v2`), not a field on `.v1`. |
| Is this an optional field on a record this repo owns and nothing external consumes yet? | Additive change to that schema plus its example. State in the commit message that it is additive and why it is safe. |

Do not add a required field to an existing `.v1` schema. Nothing in this
repository requires backward compatibility, but silently redefining a published
`$id` makes a shipped contract mean two different things.

## Step 2: move everything together

A contract change is not done until every item below is true. The first two are
enforced by tests; the rest are not, so check them yourself.

- [ ] The schema file validates and its `$id` is present.
- [ ] The paired `<name>.example.json` exists and validates against it. (The
      only schemas that legitimately ship without an example are the report
      envelopes listed in `scripts/static-analysis.sh`; if you are adding a new
      envelope, add it to that list with a comment saying why.)
- [ ] If a new schema id was introduced, a Rust constant names it and
      `tests/schema_ids.rs` sees it. That test asserts every input/record
      constant resolves to a shipped `$id`.
- [ ] `schemas/casegraphen/report-schema-aliases.json` covers any new
      operation-specific report id, or the report validates directly.
- [ ] Fixtures that carry the record were updated: the schema example, anything
      under `examples/casegraphen/`, and the golden report if the change alters
      generated output.
- [ ] The strict-parse path was exercised: a test feeds the new shape through the
      real binary, not only through a serde round-trip. `tests/command.rs` spawns
      `CARGO_BIN_EXE_casegraphen` — follow the existing pattern.
- [ ] `docs/specs/` describes the new shape if the change is normative.

## Step 3: verify

```sh
sh scripts/static-analysis.sh
```

That runs fmt, clippy with warnings as errors, the full suite (including the
Python-backed schema validation), `cargo package`, and the schema/example
inventory check. A contract change that passes locally passes CI, because CI runs
the same script.

## Traps seen before in this repository

- **A caller-declared value became a trust input.** If the new field influences a
  trust or authority decision, assume a caller will lie about it. Either the tool
  computes it (and overwrites whatever was supplied), or the decision must not
  depend on it. Evidence content hashes and evidence boundaries are both computed
  or forced for exactly this reason.
- **The report asserted something no longer true.** `metadata.tool_package` once
  claimed a path the tool had left. If your change moves or renames something the
  reports describe, the reports are part of the change.
- **The example drifted from the schema.** The pairing check exists because a
  schema was once edited without its example, and only CI noticed.

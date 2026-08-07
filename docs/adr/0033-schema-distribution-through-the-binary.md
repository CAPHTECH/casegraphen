# ADR 0033: Schema Distribution Through The Binary, Not Per-Skill Copies

## Status

Accepted on 2026-08-07. Resolves issue #111.

## Context

Consuming projects were cloning this repository just to obtain the schemas the
shipped skills instruct them to author against. `install.sh` copied exactly
three schema files into skills, all from `schemas/experimental/`
(`runtime.node_report.schema.json` into `casegraphen-audit`,
`execution.topology.v0.schema.json` into `casegraphen-design`,
`skill.orchestration_handoff.v0.schema.json` and its example into
`casegraphen-orchestrate`, the latter pair also checked into the repository
itself under `skills/casegraphen-orchestrate/references/`). Nothing from
`schemas/casegraphen/` — the 17-contract stable surface, the one a consumer is
most likely to need — shipped at all.

Grepping shipped skill text for contract identifiers found five named but not
shipped: `highergraphen.case.operation_gate_profiles.v1`,
`highergraphen.case.workflow.execution_plan.v1`,
`highergraphen.case.workflow.graph.v1`,
`highergraphen.case.workflow.worker_binding.v1`, and `native.case.space.schema.json`.
The sharpest instance, `skills/casegraphen-operate/SKILL.md`, instructs the
reader to author a gate-profile document, names its schema id, inlines a
partial example, and ships neither the schema nor the example. #106 already
showed what an inlined, unchecked example does over time: a JSON snippet in
`docs/guides/release-decision-walkthrough.md` silently drifted from
`worker.binding.schema.json` until it was missing three required fields.

The issue proposed two architectures and asked that they be weighed rather
than assumed:

- **(A) Ship more schema copies** into skills via `install.sh`, the same
  mechanism already used for three experimental schemas. Solves availability,
  but several skills reference overlapping contracts, so per-skill copies
  duplicate, and `CLAUDE.md`'s single-source rule exists precisely because a
  duplicated copy drifts — which is what happened to the orchestrate skill's
  own checked-in copy having to be kept byte-identical to
  `schemas/experimental/` by a dedicated conformance check.
- **(B) Add a `casegraphen schema` command** that emits a named contract from
  the binary, so the binary alone is sufficient and skills carry no copies.

## Decision

**(B).** A `casegraphen schema list` / `casegraphen schema get --id <id> |
--file <filename>` command pair (`src/native_cli/ops/schema.rs`) serves every
`*.schema.json` and `*.example.json` file under both `schemas/casegraphen/`
and `schemas/experimental/` — not only the ones a skill happens to name today.
Every file is embedded with `include_str!` at compile time
(`src/schema_catalog.rs`); a schema's own `$id` and an example's own `schema`
field are read back out of the embedded content itself at first use, rather
than hand-copied into a second table, so the catalog's notion of a file's
identity can never disagree with the file's own declared identity. A Rust
test (`schema_catalog::tests::catalog_matches_the_schema_trees_on_disk_exactly`)
walks both directories and proves the embedded set matches exactly what is on
disk, so a schema added without a matching catalog line fails the build, not a
consumer's `schema get` months later. Every entry carries `stability: "stable"
| "experimental"`, so the distinction the issue's acceptance criteria require
survive in what a consumer receives, not only in this repository's directory
layout.

(B) is the smaller change on the numbers, not just the cleaner one: embedding
both schema trees costs roughly 800 KiB of `include_str!` text — negligible
next to a Rust CLI binary — and needs no new dependency (`include_str!`,
`serde_json`, and `std::sync::OnceLock` are already used this way elsewhere in
this crate). Shipping (A) at the same completeness the issue asks for would
mean copying dozens of files per skill and keeping each copy in sync by hand,
which is the two-classifications-for-one-question shape `CLAUDE.md` already
forbids for decision rules, applied here to schema content. (B) removes the
duplication instead of policing it: there is exactly one copy of every schema
in this repository, and the binary — not a skill's `references/` directory —
is the only thing a consumer ever reads.

Committing to (B) meant undoing the existing instance of (A), not merely
adding an alternative beside it. `install.sh` no longer copies
`execution.topology.v0.schema.json`, `runtime.node_report.schema.json`, or
`skill.orchestration_handoff.v0.schema.json`/`.example.json`; the two files
checked into `skills/casegraphen-orchestrate/references/` are deleted; the
three skills that used to point at a bundled copy now instruct
`casegraphen schema get --id <id>` (or `--file <filename>` for an example)
instead. The design skill's `execution-topology-contract.md` rationale
document still ships as a copy — it is prose, not a JSON contract, and has no
`schema get` counterpart, so this ADR does not touch it.

**(C), required regardless of (A) or (B):** `scripts/skill-conformance.py`
gained `available_schema_identity()`, which reads every schema `$id` and every
`*.schema.json`/`*.example.json` filename from both schema trees — the same
completeness surface the Rust catalog test proves against the same
directories — and fails when skill or README text names a contract identifier
or filename outside that set. This replaces the narrower, now-defunct check
that only compared the orchestrate skill's bundled copy against its source;
the new check covers every skill and every contract, the same shape
`scripts/skill-conformance.py`'s existing CLI-surface validation already
proved worthwhile for catching an undocumented flag.

## Consequences

- A consumer with only the installed binary and skills can obtain every
  contract a skill instructs them to author against — the full stable and
  experimental schema surface — without cloning this repository.
- The stable/experimental distinction is carried on every `schema list` /
  `schema get` result, not only implied by which directory a file happened to
  live in.
- `install.sh` is smaller: three special-cased `cp` blocks are gone, and the
  remaining per-skill copy (the design rationale doc) is the only one left,
  because it is documentation with no contract identity to serve from the
  binary.
- `cargo package --locked` still builds standalone: the embedded schemas are
  files already inside this crate's own directory tree, not fetched or
  generated at build time.
- A schema or example added to either tree without a matching
  `schema_catalog.rs` entry fails `cargo test`; a skill that names a contract
  the binary cannot produce fails `scripts/skill-conformance.py --check`.
  Both failures are gates, not documentation to remember to update.

# CaseGraphen

CaseGraphen is the HigherGraphen intermediate tool for representing complex,
evidence-heavy, decision-rich work as case spaces, and for driving that work
forward under deterministic control. It lifts bounded source snapshots into a
case space, derives readiness and obstructions, checks invariants, applies
reviewed morphisms, projects lossy audience-specific views — and dispatches
accepted work to workers, validating what they report before any of it becomes
accepted state.

It was extracted from `CAPHTECH/higher-graphen` (`tools/casegraphen`) at v0.7.1
and continues as a standalone repository from 0.8.0. See
[ADR 0001](docs/adr/0001-extraction-from-higher-graphen.md).

## The control model

An LLM or agent proposes; CaseGraphen decides. Concretely:

- The append-only, hash-chained `MorphismLog` is reconstructive source data:
  genesis carries the complete initial cell/relation payload and the immutable
  case-space shell, and later entries carry reducer payloads. `space rebuild`
  folds that log from an empty case space, verifies every revision checksum,
  recreates missing periodic snapshots, and refuses to overwrite a snapshot
  that disagrees. `space replay` reads the newest snapshot at or before the
  current revision after verifying its checksum and embedded log prefix, folds
  the remaining entries, and verifies the final replay checksum. A separate
  constant-size head file pins the log tail's revision, entry hash, and replay
  checksum; a missing or stale head refuses every read. `space validate`
  additionally proves that a full streaming log fold reproduces the current
  state and checks every snapshot belonging to a logged revision. The only
  provisioning path for a missing head is the explicit operator assertion
  `space rebuild --adopt-existing-log`: it verifies the complete fold and every
  existing snapshot before creating the head, and never replaces an existing
  head.
- Readiness, the frontier, and blockers are **derived** from the loaded case
  space. They are never stored as mutable state.
- Generated structure — obstructions, completions, inferred evidence, proposed
  morphisms — is born `unreviewed` and stays that way until an explicit review
  morphism accepts it. Inferred evidence does not satisfy a hard requirement.
- A morphism applies only when its base revision matches the replayed revision
  and the required checks pass.
- Every durable mutation after the explicitly ungated genesis import requires an
  operation gate naming an actor, a capability that actually grants that actor,
  a scope, an audience, and a source boundary. Commands validate the gate before
  building a morphism, and the store validates it again at the append boundary.

The gated commands accept a named profile with
`--gate-profile <name> --gate-profile-file <path>`. The file is a strict
`highergraphen.case.operation_gate_profiles.v1` JSON document; it is selected
on each invocation and is not store state or a persistent enablement. A profile
may contain any subset of the five existing gate fields. An explicit gate flag
wins over the profile for that field, and a field missing from both is refused
as before. Validation always checks the expanded values against the replayed
case space, and `morphism.metadata.operation_gate` records those values only —
never the profile name or path. The profile cannot supply `--enable-worker`,
`--base-revision-id`, or reviewer identity. See the shipped
[`operation-gate-profiles` schema](schemas/casegraphen/operation-gate-profiles.schema.json)
and [example](schemas/casegraphen/operation-gate-profiles.example.json).

## Graph Engineering Plane

CaseGraphen now separates three graphs instead of treating runtime deployment
as accepted case meaning:

1. The stable **Case Graph / Acceptance Ledger** records goals, work,
   evidence, authority, review, revisions, and accepted change.
2. The experimental **Execution Topology v0** describes deployable nodes,
   typed handoffs, resource claims, verification/expansion policy, and the
   dependencies that constrain safe parallelism.
3. The experimental **Runtime Run Graph** is reconstructed from untrusted node
   reports, attempts, artifacts, allocations, and streaming events.

The compiler, linter, runtime reconciler, resource protocol, verification
policy, bounded expansion, streaming reconciliation, simulation, and redesign
modules connect those layers without merging their authority. Parsing,
compiling, linting, simulating, or reconciling a topology never accepts it or a
runtime claim. See the [positioning ADR](docs/adr/0002-graph-engineering-positioning.md),
[topology design](docs/design/execution-topology-contract.md), and
[experimental contract inventory](schemas/experimental/README.md).

The v0 workflow named `streaming` implements terminal-artifact stage
pipelining: a downstream stage may be proposed after its canonical producer
has terminated and its final artifact bytes are observed, without waiting for
the whole graph. It does not provide chunk-level producer/consumer overlap.

These contracts remain `experimental` and `v0`: real-runtime pilots may cause
breaking changes. They are not yet eligible for stable-schema compatibility
claims. The supported standalone experimental workflow matrix is
machine-readable in [`docs/product-surface.v0.json`](docs/product-surface.v0.json).
The durable, authenticated `casegraphen-mcp-host` delegates compile, runtime
reconciliation, simulation, resource reservation/reconciliation, bounded
expansion, streaming reconciliation, and redesign proposals to their canonical
library owners. Its persistence/authentication boundary is described in
[ADR 0019](docs/adr/0019-external-mcp-control-plane.md), and the
[operational walkthrough](docs/guides/graph-engineering-product-surface.md)
shows the review-seamed end-to-end path.

Proposal compilation remains inspection-only. The operational reviewed path
attaches and accepts the exact topology plus policy manifest, then calls
`compile_reviewed_deployment_bundle`; the host replays the accepted revision
and derives authority instead of accepting a caller-created mode or hash.
Resource reservations are available only for that content-addressed reviewed
deployment and retain its review, node, attempt, and declaration binding in
the allocator journal.

Operational resource reservations are atomic, durable, and derived from a
host-canonical append-only allocator journal. Resource-bearing runtime runs use
a versioned expectation bundle bound to the exact topology hash and observed
case revision before they can reach the review seam; neither path auto-accepts
runtime output. See [ADR 0022](docs/adr/0022-atomic-resource-allocation-and-runtime-expectations.md).

## Execution control

`run --step` advances exactly one work item per invocation. `run --frontier`
advances one whole eligible frontier round: workers run concurrently up to
`--max-parallel` (default 4), then their results append serially in plan-step
order. Neither form is a daemon, scheduler, retry engine, or event bus.

A `started` trace blocks another dispatch for its step even when unrelated
appends move the current revision. `--retry-step <step-id>` retries only a
failed attempt. Recover a dispatcher known to be dead with repeatable
`--supersede-trace <trace-id>`; each id must resolve to that plan's exact
`started` trace, and the superseding trace records the assertion in
`metadata.superseded_trace_ids`.

1. Replay and pin the revision; a stale base revision is a failure, not a merge.
2. Re-derive readiness.
3. Verify the ExecutionPlan against the plan-review morphism that accepted it
   (content-hashed), and check the dispatch gate.
4. Select the first eligible plan step for `--step`, or every eligible plan
   step for `--frontier` (at most one per work cell).
5. Verify the worker binding's content hash, canonical paths, and executable
   hash against what the plan froze.
6. Project the input for the worker and record what that projection loses.
7. Execute the worker (environment cleared to an allowlist, absolute paths,
   mandatory timeout, output capped and hashed). Frontier workers execute
   concurrently; each keeps a separately reserved run directory.
8. Attach the output as untrusted evidence — always, including on failure.
9. Apply the state transition only if the worker succeeded, the step's declared
   success requirements are satisfied by evidence from this run, the transition
   falls inside the plan's authorized transition classes, and the candidate
   post-transition state introduces no new hard obstruction.
10. Commit as a new revision and write the execution trace, anchoring its hash
    in the log. Frontier results take this application path serially in
    plan-step order, regardless of worker completion order.

Execution outcomes are domain findings — obstructions recorded in the trace —
not crashes. Stale revisions, integrity mismatches, and invalid
`--supersede-trace` assertions are tool failures.

Effectful workers are **off by default**: both run modes refuse a shell binding
unless `--enable-worker shell` is passed on that invocation. Read
[the worker execution security and approval policy](docs/security/worker-execution-policy.md)
before enabling it against a real project; it documents the threat model, the
enforced controls, what always needs a human, and the accepted residual risks.

## Install and build

```sh
cargo install casegraphen
```

From a checkout, `install.sh` installs the binary and the agent skills that
drive it in one step — see [Driving it from an agent](#driving-it-from-an-agent).
The supported compiler contract is Rust 1.80: `Cargo.toml` declares the MSRV,
`rust-toolchain.toml` pins 1.80.0 with rustfmt and Clippy, and the Quality
workflow runs the same pin. `sh scripts/static-analysis.sh` reports the active
versions and fails if the two repository declarations drift.

```sh
cargo test
cargo clippy --all-targets
```

Integration tests validate the JSON contracts with `python3 -m jsonschema`.

## Command surface

Commands declare their supported renderer explicitly. Case-space operations
use JSON by default; `space reason` also provides a text terminal projection,
and `graph lint` supports JSON or text without creating a second decision rule.

```text
casegraphen lift native|workflow|case-graph      # native and graph lifts
casegraphen lift github-issues                   # bounded GitHub issue snapshot lift
casegraphen graph lint                           # experimental topology analysis; json|text
casegraphen space new|list|inspect|history|replay|validate|reason|frontier|evidence|project|topology
casegraphen space rebuild [--adopt-existing-log]  # adoption is a human trust assertion
casegraphen morphism propose|check|apply|reject   # apply/reject are gated
casegraphen review accept|reject|reopen|waive     # gated
casegraphen evidence attach                       # gated
casegraphen cell transition                       # gated
casegraphen packet apply|resume                   # gated; resume refuses until an independent review lands
casegraphen binding register
casegraphen plan propose|check|accept|reject      # accept/reject are gated
casegraphen run --step|--frontier                 # gated; worker off by default
casegraphen obstruction list | completion candidates | projection apply
casegraphen equivalence check | invariant check|close-check
```

Domain findings exit `0` by default and remain in the report payload. For CI,
`space reason`, `obstruction list`, `invariant check`, `invariant close-check`,
`run --step`, and `run --frontier` accept `--strict`: a carried domain finding
then exits `2`, while clean reports still exit `0` and tool failures always exit
`1`. The flag changes only the exit code, never the rendered payload.

The former `workflow *` and `cg workflow *` evaluator surface was removed
(ADR 0003): `lift workflow` materializes a workflow graph into a case space,
and the native derived commands answer what those commands answered.

`casegraphen` with no arguments prints the full usage text.

`evidence attach` may repeat the positional group `--input <path>
[--satisfies <target-id>]... [--artifact <path>]...`. Each `--satisfies` and
`--artifact` belongs to the most recent `--input`. `--artifact` names a file to
record as the immutable observation the claim is about: the tool hashes it,
mints a content-addressed `custom:artifact` cell (or reuses the one already
recorded for that hash), and adds a `derives_from` relation from the claim.
Review lands on the claim, not on the artifact it cites. One invocation
validates and normalizes every input before appending one morphism and one
revision; a refusal in any group appends nothing. A single group retains the
original command and report shape.

## Driving it from an agent

Four separately constrained Skills ship under [`skills/`](skills/):

- `casegraphen-design` creates linted, unreviewed topology proposals and never
  mutates, reviews, accepts, or runs them.
- `casegraphen-audit` performs read-only static/run audits and preserves the
  distinction between deterministic findings and review-required inference.
- `casegraphen-integrate` imports generic JSONL runtime artifacts and reports
  as untrusted observations, reconciles them through the canonical library,
  and stops at the review seam.
- `casegraphen-operate` owns the revision/gate/refusal protocol for mutations
  of the acceptance ledger.

[`install.sh`](install.sh) installs the binary and the skill together, because a
skill for a CLI is useless without the CLI it documents:

```sh
sh /path/to/casegraphen/install.sh          # skills go to ~/.claude and ~/.codex
```

See [`skills/README.md`](skills/README.md) for what it writes where, and for the
other ways to install the skill.

## Contracts

Stable wire formats live in [`schemas/casegraphen/`](schemas/casegraphen/) and
are strict: unknown fields are rejected and breaking changes require a new
schema id. Experimental Graph Engineering Plane proposals live separately in
[`schemas/experimental/`](schemas/experimental/) and may change incompatibly
while carrying their `v0` identities; they are inventoried and tested without
being promoted. The one deliberate exception in the stable set is a record that mirrors another system's
output rather than stating something to this tool — the `gh --json` objects
inside the GitHub issue snapshot, whose field set is GitHub's to grow. Those
accept unknown fields and declare them as information loss; the CaseGraphen
wrapper around them does not. The case space (`highergraphen.case.space.v1`) and
workflow graph (`highergraphen.case.workflow.graph.v1`, a lift input since
ADR 0003) are inputs; reports and the execution records (`execution_plan`,
`worker_binding`, `worker_report`, `execution_trace`) are versioned alongside
them.

## Relationship to HigherGraphen

CaseGraphen depends on the published `higher-graphen-core`,
`-structure`, and `-reasoning` crates and must not depend on
`higher-graphen-runtime`; runtime reports may be consumed as evidence input JSON
only. No HigherGraphen crate depends on CaseGraphen. Specifications owned by
this tool live in [`docs/specs/`](docs/specs/).

## Documents

- [Walkthrough: deciding a release with CaseGraphen](docs/guides/release-decision-walkthrough.md) —
  one case space driven from lift to a refused close, with the executed commands
  and their output
- [Independence and execution control design](docs/design/independence-and-execution-control.md)
- [ADR 0001: extraction and execution-control mandate](docs/adr/0001-extraction-from-higher-graphen.md)
- [ADR 0002: positioning within graph engineering](docs/adr/0002-graph-engineering-positioning.md) —
  CaseGraphen is the acceptance ledger of a graph-engineered system, not its
  runtime
- [Execution Topology contract](docs/design/execution-topology-contract.md) and
  [resource-reservation protocol](docs/design/resource-reservation-protocol.md)
- [ADR 0019: external MCP control-plane boundary](docs/adr/0019-external-mcp-control-plane.md)
- [ADR 0020: Graph Engineering product surface](docs/adr/0020-graph-engineering-product-surface.md)
- [ADR 0024: deterministic streaming order](docs/adr/0024-streaming-events-have-logical-order.md)
- [ADR 0025: runtime edge-handoff completeness](docs/adr/0025-runtime-completeness-requires-edge-handoffs.md)
- ADR identifiers are contiguous, immutable four-digit decision identities;
  filenames and headings carry the same identifier. The next available
  identifier is **0026**. [ADR 0012](docs/adr/0012-adr-identifier-inventory.md)
  defines the inventory convention enforced by the release gate.
- [Fresh-agent release evaluation](docs/guides/fresh-agent-release-eval.md) —
  the ten-scenario harness, real-provider matrix, captured evidence, and
  stable-promotion threshold. A [retained Codex/Claude smoke report](docs/evals/fresh-agent/2026-08-03-real-provider-smoke.md)
  demonstrates the review seam, but is explicitly not the full promotion matrix
- [ADR 0007: is a capability scoped to an operation?](docs/adr/0007-capability-operation-scope.md) —
  proposed; the gate checks who holds a capability, not what it authorizes
- [Worker execution security and approval policy](docs/security/worker-execution-policy.md)
- [Authorization and evidence-coverage audit](docs/audit/authorization-and-evidence-coverage-2026-07-31.md) —
  reproduced routes that clear a hard evidence requirement without a review

## License

Apache-2.0. Copyright 2026 CAPH TECH Inc. See [LICENSE](LICENSE) and
[NOTICE](NOTICE).

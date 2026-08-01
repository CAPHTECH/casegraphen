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

## Execution control

`run --step` advances exactly one work item per invocation. `run --frontier`
advances one whole eligible frontier round: workers run concurrently up to
`--max-parallel` (default 4), then their results append serially in plan-step
order. Neither form is a daemon, scheduler, retry engine, or event bus.

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

Anything that fails is a domain finding — an obstruction recorded in the trace —
not a crash. Only stale revisions and integrity mismatches are tool failures.

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

```sh
cargo test
cargo clippy --all-targets
```

Integration tests validate the JSON contracts with `python3 -m jsonschema`.

## Command surface

All commands take `--format json` and optionally `--output <path>`.

```text
casegraphen lift native|workflow|case-graph      # native and graph lifts
casegraphen lift github-issues                   # bounded GitHub issue snapshot lift
casegraphen space new|list|inspect|history|replay|validate|reason|frontier|evidence|project|topology
casegraphen space rebuild [--adopt-existing-log]  # adoption is a human trust assertion
casegraphen morphism propose|check|apply|reject   # apply/reject are gated
casegraphen review accept|reject|reopen|waive     # gated
casegraphen evidence attach                       # gated
casegraphen cell transition                       # gated
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
`1`. The flag changes only the exit code, never the JSON payload.

The former `workflow *` and `cg workflow *` evaluator surface was removed
(ADR 0003): `lift workflow` materializes a workflow graph into a case space,
and the native derived commands answer what those commands answered.

`casegraphen` with no arguments prints the full usage text.

## Driving it from an agent

[`skills/casegraphen-operate`](skills/) is an agent skill for operating a case
space: the revision and gate discipline every mutating command needs, how to
model readiness so it comes out right, how to read the refusals, and — when an
agent runtime executes the graph — what to record as evidence and at which
granularity.

[`install.sh`](install.sh) installs the binary and the skill together, because a
skill for a CLI is useless without the CLI it documents:

```sh
sh /path/to/casegraphen/install.sh          # skills go to ~/.claude and ~/.codex
```

See [`skills/README.md`](skills/README.md) for what it writes where, and for the
other ways to install the skill.

## Contracts

Wire formats live in [`schemas/casegraphen/`](schemas/casegraphen/) and are
strict: unknown fields are rejected and breaking changes require a new schema
id. The one deliberate exception is a record that mirrors another system's
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
- [ADR 0007: is a capability scoped to an operation?](docs/adr/0007-capability-operation-scope.md) —
  proposed; the gate checks who holds a capability, not what it authorizes
- [Worker execution security and approval policy](docs/security/worker-execution-policy.md)
- [Authorization and evidence-coverage audit](docs/audit/authorization-and-evidence-coverage-2026-07-31.md) —
  reproduced routes that clear a hard evidence requirement without a review

## License

Apache-2.0. Copyright 2026 CAPH TECH Inc. See [LICENSE](LICENSE) and
[NOTICE](NOTICE).

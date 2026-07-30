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
  recreates missing snapshots, and refuses to overwrite a snapshot that
  disagrees. `space replay` itself reads the current snapshot after verifying
  its checksum and embedded log prefix; `space validate` additionally proves
  that a full log fold reproduces the current snapshot.
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

`run --step` advances exactly one work item per invocation. There is no daemon,
scheduler, retry engine, or event bus.

1. Replay and pin the revision; a stale base revision is a failure, not a merge.
2. Re-derive readiness.
3. Verify the ExecutionPlan against the plan-review morphism that accepted it
   (content-hashed), and check the dispatch gate.
4. Select the first plan step that is on the frontier and not yet executed.
5. Verify the worker binding's content hash, canonical paths, and executable
   hash against what the plan froze.
6. Project the input for the worker and record what that projection loses.
7. Execute the worker (environment cleared to an allowlist, absolute paths,
   mandatory timeout, output capped and hashed).
8. Attach the output as untrusted evidence — always, including on failure.
9. Apply the state transition only if the worker succeeded, the step's declared
   success requirements are satisfied by evidence from this run, the transition
   falls inside the plan's authorized transition classes, and the candidate
   post-transition state introduces no new hard obstruction.
10. Commit as a new revision and write the execution trace, anchoring its hash
    in the log.

Anything that fails is a domain finding — an obstruction recorded in the trace —
not a crash. Only stale revisions and integrity mismatches are tool failures.

Effectful workers are **off by default**: `run --step` refuses a shell binding
unless `--enable-worker shell` is passed on that invocation. Read
[the worker execution security and approval policy](docs/security/worker-execution-policy.md)
before enabling it against a real project; it documents the threat model, the
enforced controls, what always needs a human, and the accepted residual risks.

## Install and build

```sh
cargo install casegraphen
```

```sh
cargo test
cargo clippy --all-targets
```

Integration tests validate the JSON contracts with `python3 -m jsonschema`.

## Command surface

All commands take `--format json` and optionally `--output <path>`.

```text
casegraphen lift native|workflow|case-graph      # bounded lift into a case space
casegraphen space new|list|inspect|history|replay|rebuild|validate|reason|frontier|topology
casegraphen case  ... |obstructions|completions|evidence|project|close-check
casegraphen morphism propose|check|apply|reject   # apply/reject are gated
casegraphen review accept|reject|reopen|waive     # gated
casegraphen evidence attach                       # gated
casegraphen cell transition                       # gated
casegraphen binding register
casegraphen plan propose|check|accept|reject      # accept/reject are gated
casegraphen run --step                            # gated; worker off by default
casegraphen obstruction list | completion candidates | projection apply
casegraphen equivalence check | invariant check|close-check
casegraphen workflow reason|validate|readiness|obstructions|completions|evidence|project|correspond|evolution
casegraphen cg workflow ...                       # store-backed workflow bridge
```

`casegraphen` with no arguments prints the full usage text.

## Contracts

Wire formats live in [`schemas/casegraphen/`](schemas/casegraphen/) and are
strict: unknown fields are rejected and breaking changes require a new schema
id. The case space (`highergraphen.case.space.v1`) and workflow graph
(`highergraphen.case.workflow.graph.v1`) are inputs; reports and the execution
records (`execution_plan`, `worker_binding`, `worker_report`, `execution_trace`)
are versioned alongside them.

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
- [Worker execution security and approval policy](docs/security/worker-execution-policy.md)

## License

Apache-2.0. Copyright 2026 CAPH TECH Inc. See [LICENSE](LICENSE) and
[NOTICE](NOTICE).

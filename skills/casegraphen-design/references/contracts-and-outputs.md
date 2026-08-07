# Contracts and output boundaries

Read this before writing proposal artifacts.

## Stability map

| Artifact | Contract status | Meaning |
|---|---|---|
| `execution.topology.json` | Experimental `casegraphen.experimental.execution.topology.v0` | Proposed deployable graph shape; validation and hashing do not accept it |
| `graph.analysis.report.json` | Experimental graph-lint report | Deterministic analysis of a proposal; never a review or transition |
| `genesis.case.space.json` | Stable native case-space contract when schema-valid | Authored genesis input; lifting it is a separate operator action |
| `execution.plan.json` | Stable execution-plan contract when schema-valid | Proposed plan; acceptance is a separate gated review action |
| `genesis.mapping.proposal.md` | Documentation only | Proposed mapping from acceptance units to stable genesis fields |
| `execution-plan.mapping.proposal.md` | Documentation only | Proposed mapping from topology nodes to stable plan steps |
| `verification.policy.json` | Design metadata until separately contracted | Intended producer/verifier constraints and anchors |
| `runtime.deployment.json` | Runtime-owned metadata | Executor, isolation, routing, and deployment choices |

In a repository checkout, the authoritative v0 field definitions live in
`schemas/experimental/execution.topology.v0.schema.json`, and the contract
rationale lives in `docs/design/execution-topology-contract.md`. Without a
checkout, get the same schema straight from the installed binary:

```sh
casegraphen schema get --id casegraphen.experimental.execution.topology.v0 --format json
```

A user-level installation still includes a byte-for-byte copy of the contract
rationale at `references/execution-topology-contract.md` (there is no
`schema get` command for prose). Read the applicable installed or checkout
file directly; do not manually copy enums or validation rules into
instructions.

## Mapping proposal content

Both mapping proposals must name:

- `case_space_id`;
- `observed_revision_id` exactly as supplied or inspected;
- source acceptance unit and proposed runtime node/plan step;
- information not representable without loss;
- validation or review still required;
- the fact that no mutation or acceptance was performed.

If the case space moves after observation, report the stale basis. Do not update
`observed_revision_id`, regenerate against a different revision, or claim the
proposal applies to the new head without a new explicit design pass.

## Output discipline

Write all artifacts to a new or caller-approved directory. Do not place them in
the CaseGraphen store. Content-addressing, compilation, deployment, evidence
attachment, and acceptance are later workflows with separate authority.

# Operating the experimental Memory Plane

The Memory Plane reads accepted Case Graph state and emits source-bound
proposals. It has no accepted-write command.

## Prepare and inspect a source

Hash and retain exact bytes outside any summary. Author strict
`memory.source_record.v0` and `memory.claim.v0` documents, then run:

```sh
casegraphen memory source inspect \
  --source-record memory.source.json \
  --source-artifact source.bin \
  --format json

casegraphen memory check \
  --input memory.claim.json \
  --source-record memory.source.json \
  --source-artifact source.bin \
  --policy memory.policy.json \
  --format json
```

Both are non-mutating. `source attach` also emits only a content-addressed
attachment proposal. It does not copy bytes into a CaseStore.

## Propose a claim

```sh
casegraphen memory propose \
  --store case-store \
  --case-space-id case_space:project \
  --input memory.claim.json \
  --source-record memory.source.json \
  --source-artifact source.bin \
  --policy memory.policy.json \
  --format json
```

The CLI replays that exact CaseSpace, rejects a mismatched
`claim.scope.case_space_id`, and derives the cell's `space_id` from the replay.
Require lifecycle `proposed`, review status `unreviewed`, `accepted: false`, and
`mutation_performed: false`. To persist it, an operator must deliberately map
the proposal to the existing content-addressed artifact/evidence and gated
morphism workflow. The proposer must not perform that review.

The operational host exposes `memory_propose_claim`,
`memory_propose_supersession`, `memory_propose_retraction`, and
`memory_propose_procedure`. Each request binds an exact current revision and a
source artifact path confined beneath the host artifact directory. They return
unreviewed structures only.

## Query accepted memory

First replay the case and copy its exact `current_revision_id` into
`memory.query.v0`. Then:

```sh
casegraphen memory query \
  --store case-store \
  --case-space-id case_space:project \
  --input memory.query.json \
  --policy memory.policy.json \
  --format json
```

Use `memory explain|history|sources --target-id <claim-id>` for one claim,
`memory conflicts` for the contested set, and `memory candidates` for explicit
historical/candidate inspection. Do not use the latter as current agent
instructions.

`memory sources` returns the strict Source Record contracts for an authorized,
selected claim alongside its content-addressed artifact references. It never
returns source records for a claim filtered by scope, sensitivity, authority,
time, or review state; exact bytes remain in the governed artifact boundary.

The corresponding operational tools are `memory_query`, `memory_explain`,
`memory_history`, `memory_conflicts`, and `memory_sources`. All require
`base_revision_id`, return `read_only: true`/`mutation_performed: false`, and
still apply the actor grant inside the typed Memory Query. Bearer access and
caller-declared MCP audit context do not replace that grant.

## Rebuild indexes

```sh
casegraphen memory index rebuild \
  --store case-store \
  --case-space-id case_space:project \
  --input memory.query.json \
  --policy memory.policy.json \
  --format json \
  --output memory.index.json

casegraphen memory index validate \
  --store case-store \
  --case-space-id case_space:project \
  --input memory.query.json \
  --policy memory.policy.json \
  --index memory.index.json \
  --format json
```

An index is valid only when replay produces the same content hash and it states
`derived: true`, `authoritative: false`. Delete and rebuild it whenever the
bound revision, policy, or query changes.

## Failure handling

- `stale_revision`: replay, review the new state, and create a new query or
  proposal; never substitute a revision silently.
- authority or scope finding: use a properly authorized reviewer/policy; do not
  widen a grant merely to pass.
- expired/superseded/retracted: use only in an explicit historical query.
- hard conflict: keep both contested claims visible and stop current use until
  a reviewed resolution lands.
- source/hash mismatch: recover the exact bytes; do not rewrite the recorded
  hash to match a different artifact.

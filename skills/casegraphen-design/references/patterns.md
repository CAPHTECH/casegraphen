# Topology patterns

Use these as modeling shapes, then let `graph lint` judge the concrete graph.

## Independent fan-out

Give each independently executable item its own node, typed input, output, and
resource claims. Add no ordering edge merely to express list order. If two
nodes touch a shared exclusive resource, independence is false and the resource
relationship must be explicit.

## Reduction and synthesis

Separate collection from judgment:

```text
fan-out producers -> bounded reducers -> synthesis -> verification
```

Use multiple reduction levels when one consumer would receive an unbounded or
context-heavy fan-in. Keep reducer inputs and outputs typed. A synthesis node
combines reduced results; it is not an excuse to erase artifact lineage.

## Barrier versus streaming

Use a barrier when a consumer requires a complete input set or a stable review
boundary. Use streaming only when partial delivery has defined semantics in the
runtime contract. Delivery mode does not grant the runtime permission to append
or accept CaseGraphen state.

## Resource isolation

Declare file, worktree, API quota, database, fixture, branch, network, and
secret scopes that affect safe concurrency. Isolated worktrees reduce workspace
collision but do not make conflicting edits semantically mergeable; preserve
the common resource identity so the linter can analyze it.

## Dynamic expansion

Treat discovered nodes and edges as new unreviewed topology proposals. Define
deduplication over all seen candidates, dry-round termination, iteration/node
limits, and cost limits in the referenced expansion policy. A runtime discovery
never directly edits accepted topology.

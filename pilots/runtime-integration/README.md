# Local runtime integration pilots

These pilots exercise the experimental Graph Engineering Plane through the
durable `casegraphen-mcp-host`. They are release evidence, not a scheduler and
not an acceptance path.

Run them with the repository-pinned Rust toolchain and Python 3:

```sh
cargo build --bin casegraphen-mcp-host
python3 scripts/runtime-integration-pilots.py \
  --repo . \
  --host-bin target/debug/casegraphen-mcp-host \
  --output /tmp/casegraphen-runtime-pilot
```

The integration test runs the same command with Cargo's freshly built host:

```sh
cargo test --test runtime_pilots -- --nocapture
```

## Runtime boundaries

| Adapter | Native runtime behavior | Boundary conversion |
|---|---|---|
| `generic-jsonl` | Two local subprocesses fan out, one fails and is explicitly retried, then a local reducer runs | Emits artifacts and `runtime.node_report.v0` envelopes directly |
| `file-drop` | Two local subprocesses commit the same relative filename in physically distinct Git worktrees and each drops its own native report | Reads the native files and output bytes before producing generic JSONL envelopes |

The adapters are materially different before the CaseGraphen boundary. They
share the generic JSONL reconciliation contract intentionally, so the canonical
completeness and review rules remain single-sourced.

## Scenario matrix

The harness checks a complete fan-out/reduce run, an explicit retry chain, a
missing reducer report, a reducer output-schema mismatch, an unsafe shared-file
topology, and isolated-workspace reports whose resource contracts are absent at
the host boundary. The last case must halt
`resource_reconciliation_incomplete`; runtime-declared `worktree_id` and
`commit_sha` never stand in for a reservation.

Inputs, topology, deployment, and output bytes are hashed. Runtime identity,
version, latency, token/cost, worktree, commit, and deployment values remain
untrusted declarations. Every reconciliation result has `accepted: false`; a
complete result stops at `needs_review` and emits only unreviewed proposals.

The checked-in run under `docs/pilots/issue-58/` is one observation. Measured
latencies and the local Python version can differ on a later machine. The test
asserts the protocol invariants rather than treating those declarations as a
golden truth.

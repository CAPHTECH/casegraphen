# Experimental graph-engineering contracts

Contracts in this directory are proposals used to validate vocabulary and
integration boundaries before promotion into CaseGraphen's stable schema
registry. A `v0` contract may change incompatibly after review of real runtime
integrations.

`contracts.v0.json` is the fail-closed inventory for this experimental surface.
It binds every shipped `$id` to one public Rust schema constant, classifies the
contract as input, record, or generated report, lists its examples and
cross-contract dependencies, and records the narrow exemptions for reports
whose fixtures are produced by Rust tests. `scripts/experimental-schema-conformance.py`
validates that inventory, every example, external `$ref`, and Rust-serialized
representative instances. Its negative fixtures prove that duplicate IDs,
orphan files, stale versions, and unresolved references cannot silently pass.

Passing this gate means the checked-out experimental contracts agree with one
another. It does **not** grant stable compatibility: only contracts promoted to
`schemas/casegraphen` enter the stable schema policy. Experimental `v0` IDs may
still change incompatibly before promotion, but any such change must update its
Rust owner, examples, references, and inventory atomically.

- `runtime.stream_event.v0.schema.json` defines stable attempt/sequence and
  logical-order observations for idempotent external streaming reconciliation.
  Its early-release outputs are proposals, never accepted log entries.

- `verification.policy.v0.schema.json` separates ledger-verifiable
  constraints, runtime attestations, and properties that are not observable by
  CaseGraphen. Its reconciliation result is not evidence acceptance.

- `runtime.node_report.schema.json` defines
  `casegraphen.experimental.runtime.node_report.v0`. Every value is an
  untrusted external-runtime observation. In particular, runtime identity,
  model, context, status, freshness, usage, cost, and allocation declarations
  are not accepted facts and cannot satisfy evidence or verification policy by
  themselves.
- `runtime.node_report.example.json` is a round-trip fixture for the Rust
  validation and canonicalization implementation.
- `runtime.integration.jsonl-record.v0.schema.json` defines the strict generic
  JSONL envelopes consumed by `GenericJsonlReconciler`. Artifact content is
  UTF-8 text in v0 and must match its content-addressed identifier.
- `expansion.policy.v0.schema.json` bounds dynamic discovery with all-seen
  deduplication, consecutive dry rounds, iterations, spawned nodes, and cost.
  Discoveries remain content-addressed unreviewed topology/morphism proposals.
- `topology.redesign_proposal.v0` and its disposition log retain exact
  canonical node/edge/policy changes, audit and simulation artifacts, review
  uncertainty, and append-only proposed/rejected/superseded/accepted-binding
  history. An accepted binding is not a topology mutation API.
- `execution.topology.v0.schema.json` describes unreviewed deployable graph
  shape, typed handoff, non-data dependencies, and resource claims. Its two
  examples cover independent file review/reduction, code-changing worktree
  nodes with a shared-file collision, and the installed design fixture covers
  a bounded hierarchical reduction of 1,000 source items.
- `graph_lint.report.v0.schema.json` separates contract violations,
  deterministic warnings, and heuristics; suggested next operations are data,
  never assembled shell commands.
- `graph_simulation.request.v0.schema.json` and
  `graph_simulation.report.v0.schema.json` define bounded, seeded what-if
  simulation over an exact topology hash. Missing calibration remains an
  explicit unknown, and routing output remains an unreviewed proposal.
- `resource.declaration.v0`, `resource.reservation.v0`,
  `runtime.resource_allocation.v0`, and `resource.reconciliation.v0` keep
  topology claims, attempt grants, untrusted actual allocations, and their
  deterministic comparison separate. Reservation disposition and rate-limit
  capacity are explicit records; elapsed time never releases a reservation.
- `git.worktree_record.v0` is the reference isolation record. The Rust
  worktree adapter creates and removes only explicitly located isolated
  worktrees from an exact base commit; integration fixtures use disposable
  repositories, and cleanup requires a matching release/supersede assertion.

Runtime reports join to `execution.topology.v0` only through the exact topology
identifier/content hash and node identifier. Completeness reconciliation is a
diagnostic; it does not append evidence, accept a claim, or transition a case.
Case graph meaning, execution topology, and actual runtime history remain
separate. Parsing, linting, hashing, compiling, or reconciling an experimental
artifact never makes it accepted.

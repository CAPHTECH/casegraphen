# Issue 52 implementation local-optima audit

## 1. Executive summary

- Scope: ADR 0019, transport-neutral control-plane state, external stdio MCP
  binary, catalog/schemas, idempotency/replay, notifications, and delegate
  boundary.
- Outcome: support long-lived graph clients without introducing a daemon or a
  second CaseGraphen decision engine.
- Conclusion: no high-severity local optimum remains in core. The main bounded
  externalization is durable idempotency: the reference state is in-memory, so
  an external server must persist its request/idempotency/notification records
  transactionally across process restarts.
- Evidence limit: deterministic tests and real child-process stdio transcripts
  exist; there is no crash-safe store, deployed network transport,
  authentication system, or production reconnect metric.

## 2. Evaluation conditions

| Variable | Current condition | Expanded condition |
|---|---|---|
| `B` | core protocol library and reference stdio child process | persistent wrapper/store, network clients, runtime and ledger |
| `M` | deterministic replay and decision delegation | crash-safe exactly-once effects, availability, auth, operational latency |
| `N` | ADR/module/schemas/tests | external server/store/transport and all decision owners |
| `T` | one connection/reconnect session | process restart, upgrades, concurrent clients, schema evolution |

The local design optimizes separation: protocol state is allowed in core;
transport lifecycle, persistence, scheduling, retries, and model execution are
not.

## 3. Evidence planes

| Plane | Evidence | Observation | Limit |
|---|---|---|---|
| Structural | `src/control_plane.rs`, ADR 0019 | The module owns catalogs, identity, replay and notifications; a delegate owns decisions. | Static only. |
| Execution | `tests/control_plane.rs`, `tests/mcp_stdio.rs` | Library tests plus protocol transcripts cover initialize, discovery, reads/calls, stale refusal, idempotent reconnect, notification replay, and a real stdio child process. | No crash injection or durable store. |
| Evolution | Rust constants versus schema `const` compatibility test | Drift across catalog and wire schema fails CI. | No release history. |
| Meaning/organization | ADR 0002/0019 | External package owns trust, connection, persistence, and atomicity; notifications grant no authority. | No package owner or SLO observed. |

## 4. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | In-memory idempotency/replay state, now exposed by a real stdio binary | no dependency, daemon, or persistence in core | binary restart loses protocol history; production wrapper must persist atomically | process restart | 10 | C3 | `externalization` |
| 2 | Manually enumerated catalog in Rust/schema | explicit compatibility surface | coordinated additions in two files | repeated protocol growth | 4 | C2 | `time-delayed` |

## 5. Candidate LO-52-1: in-memory protocol state

### Facts, inference, hypothesis

- **Observed:** `ControlPlaneState` deduplicates request ids, semantic
  idempotency keys, notifications, and replays by sequence.
- **Observed:** reconnect with a new request id and the same semantic key calls
  the delegate once; collisions fail closed.
- **Observed:** the reference `casegraphen-mcp` process exits at stdin EOF and
  constructs fresh in-memory state at startup.
- **Observed:** ADR 0019 assigns persistence/atomicity to the external package.
- **Inference:** core remains dependency-free, while crash-safe reconnect cost
  is borne by the server integrator.
- **Inference:** a wrapper that fails to persist the state before acknowledging
  a delegated mutation can duplicate ingestion/evidence/reservations after
  restart. The restart boundary is now directly observable even though the
  reference default delegate fails closed and cannot produce that side effect.

### Local rationality and compensation halo

- Local goal: transport-neutral deterministic behavior consistent with ADR
  0002.
- Beneficiary: core maintainers and all transports sharing one wire contract.
- Valid constraint: adding a database/daemon here would move runtime liveness
  into the acceptance kernel.

| Local choice | Boundary effect | Compensation | Cost bearer | Evidence |
|---|---|---|---|---|
| memory maps | state disappears on restart | transactional external persistence | server package | ADR/static |
| delegate trait | no bundled operation router | adapter wires existing CLI/library owners | integrator | lint equivalence test |

### Boundary inversion and counterfactuals

| Boundary | Current | Alternative | Advantage |
|---|---|---|---|
| module/test | simple and deterministic | database/transport coupling | current |
| one stdio process | idempotent replay, tested over reconnect | extra I/O | current |
| process restart | state loss unless persisted | durable transaction log | alternative |
| organization | adapter owns operations | core owns daemon/SLO | current architecture |

- Minimum inversion boundary: acknowledged mutation followed by server crash.
- Inverting metric/time: crash-safe exactly-once behavior over process lifetime.

- **A current:** external package persists the protocol state and delegates
  decisions. Lowest core coupling, real integration burden.
- **B local mutex/file dump:** appears easy but cannot supply multi-process
  transactionality and risks acknowledging before durable write; reject.
- **C external durable adapter:** atomically store semantic key, delegated
  result/refusal, cursor, and notification before acknowledgment. Adds store
  migration, availability and rollback operations; core wire types remain
  unchanged.

Scores: `E=3`, `A=2`, `F=2`, `K=1`, `T=2`, **Severity 10/15**,
**Confidence C3**. Verdict: `externalization`, explicit and bounded by ADR
0019. It is not repaired inside core because that would violate ADR 0002; the
external package must treat durability as a release gate.

## 6. Candidate LO-52-2 and rejected signals

The Rust catalog and JSON Schema repeat names, locally making discovery cheap
but creating coordinated edits. Exact compatibility tests convert silent drift
into a failing change. Scores: `E=0`, `A=2`, `F=0`, `K=0`, `T=2`, severity
4/15, confidence C2, `time-delayed`.

Rejected as local optima:

- Requiring revision/gate on runtime ingest and reservations is stricter than
  some CLI paths, but fails safe and preserves one client-observed concurrency
  context; delegates still decide authorization.
- A generic delegate may look incomplete, but embedding twelve routers would
  duplicate package-specific parsing and decisions. ADR 0019 deliberately
  makes wiring an external responsibility.
- Notifications set `authorizes_action=false` even when callers send true;
  this is a trust-boundary invariant, not redundant work.

## 7. Next evidence

1. Wrap the reference stdio boundary with a transactional state store and crash
   it at every acknowledge boundary, including between delegated mutation and
   protocol-state persistence.
2. Compare CLI and adapter results for every shared mutating operation, not
   only lint, using the same fixture stores.
3. Exercise concurrent reconnect and semantic-key collision across processes.
4. Measure catalog/schema co-change after several releases before introducing
   code generation.

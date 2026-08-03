# Issue 72 implementation local-optima audit

## 1. Executive summary

- Scope: the operational MCP resource allocator introduced by issue 72.
- Conclusion: caller-owned allocator state and replace-in-place persistence
  would have been harmful local optima, but the implementation removed both.
  The remaining append-only full replay is a low-severity `time-delayed`
  candidate whose inversion has not yet been observed.
- High-confidence material candidates remaining: zero.
- Evidence constraint: tests establish deterministic behavior, not production
  journal size, replay latency, filesystem diversity, or operator workload.

## 2. System outcome and B/M/N/T

System outcome: no conflicting or over-capacity grant is issued across
concurrent hosts or restarts, while reservation history remains auditable.

| Variable | Current condition | Expanded condition used by this audit |
|---|---|---|
| `B` boundary | one allocator operation and journal directory | MCP host, resource protocol, concurrent hosts, operators, runtime integration |
| `M` metric | simple deterministic append/replay and correctness | conflict rate, crash recovery, replay latency, operational repair and evolution cost |
| `N` change scope | allocator module plus host adapter | schemas, protocol, host, tests, deployment configuration and future checkpoint tooling |
| `T` horizon | one experimental-v0 request/restart | repeated concurrency, long-lived journals, capacity-policy changes and product lifetime |

Constraints: Rust 1.80, fail-closed authority boundaries, deterministic replay,
filesystem atomicity, and no independent resource decision rule in the host.

## 3. Evidence

| Observation plane | Source | What it establishes | Constraint |
|---|---|---|---|
| Structure | `src/resource_allocator.rs`, `src/resource_protocol.rs` | host state is reconstructed from hash-chained events; decisions delegate to the protocol | static inspection only |
| Execution | `tests/resource_allocator.rs` | concurrent exclusivity, capacity, lifecycle idempotency, restart and crash-boundary behavior | local filesystems and test scale |
| Evolution | experimental v0 schemas and ADR 0022 | journal/configuration are explicitly versioned and owned | no long-term change history yet |
| Meaning/organization | strict MCP payload and host configuration | callers request grants but do not author the active set or capacity | operator workflow not observed in production |

Facts below are marked `[Evidence]`; consequences not directly measured are
marked `[Inference]` or `[Hypothesis]`.

## 4. Ranked candidates

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Full journal replay on every operation | one authoritative representation and simple recovery | eventual replay latency and storage/inspection burden | long-lived operational lifecycle, not yet observed | 3/15 | C2 | `time-delayed` |
| 2 | Separate control-plane and allocator idempotency records | isolates protocol acknowledgement from allocation authority | ambiguous acknowledgement requires explicit operator reconciliation | crash between allocator commit and protocol acknowledgement | 4/15 | C2 | `harmless-locality` under fail-closed policy |

Historical alternatives—caller-supplied active sets/capacities and mutable
replace-in-place snapshots—are not current candidates because issue 72 removed
them before release.

Secondary candidate score: separate protocol/allocator idempotency is
`E=1, A=1, F=1, K=0, T=1`, `Severity=4/15`, `Confidence=C2`. `F=1` represents
the deliberately surfaced rare ambiguous-effect boundary under the tested crash
case, not a duplicated grant; fail-closed recovery keeps the classification
`harmless-locality`.

## 5. Detailed candidate card: full journal replay

### Identification and fact/inference/hypothesis

- Target/owner: `AtomicResourceAllocator::replay`, Graph Engineering host.
- [Evidence] each operation sorts and reads every published `.json` event.
- [Evidence] unpublished `.tmp` files are ignored; malformed published events
  refuse; replay re-evaluates protocol capacity/conflict semantics.
- [Inference] operation latency grows with retained event count.
- [Hypothesis] production journals will become large enough for replay latency
  or operator inspection cost to outweigh the simplicity benefit.

### Local rationality and B/M/N/T

- Local purpose/metric: deterministic authority reconstruction with the least
  number of state representations.
- Beneficiaries: host implementers, auditors, and crash recovery.
- Still-valid constraints: fail closed, deterministic history, experimental v0.
- No expired constraint is evidenced yet.
- `B`: journal module; `M`: integrity/simplicity; `N`: allocator only; `T`: one
  request/restart. Expansion to lifecycle latency creates the candidate.

### Compensation halo

| Local decision | Boundary effect | Compensation | Cost bearer | Frequency/scale | Evidence |
|---|---|---|---|---|---|
| replay all events | increasing I/O and validation work | currently none; future monitoring/checkpoint may be needed | host operator and clients | proportional to journal length | structural inference; no production measurement |
| fail closed on changed capacity config | startup/operation can refuse after policy change | explicit migration or restoration of prior config | operator | only on configuration change | replay implementation and ADR 0022 |

### Four observation planes

- Structure: a single event authority avoids snapshot/event divergence.
- Execution: crash and concurrency tests pass; no large-journal benchmark exists.
- Evolution: schemas are v0, so a checkpoint contract remains possible.
- Meaning/organization: operators own capacity configuration and any migration;
  callers cannot weaken it.

### Boundary expansion and inversion

| Boundary | Current benefit | Current cost | Checkpoint alternative benefit | Alternative cost | Advantage |
|---|---|---|---|---|---|
| Function | straightforward total replay | repeated reads | more complex read path | checkpoint validation | current |
| Module | one authority representation | O(events) work | bounded replay suffix | snapshot/event coupling | current at observed scale |
| Feature | deterministic reserve/release | possible latency growth | predictable latency | checkpoint publication path | unresolved |
| System | auditable recovery | shared storage I/O | less I/O | new corruption/fork mode | unresolved |
| Operations | simple backup of events | long inspection/recovery | quicker recovery | compaction/runbook burden | alternative only if measured threshold exceeded |
| Lifecycle | stable semantics | unbounded accumulation | bounded steady state | migration/version maintenance | probable inversion at unknown scale |

- Minimum inversion boundary: lifecycle, conditional on measured journal scale.
- Inverting metric: replay/recovery latency and operator cost.
- Inverting horizon: not established; requires operational measurements.

### A/B/C counterfactual and migration valley

#### A. Maintain current full replay

- Steady state: simplest integrity model; linear replay cost.
- Future cost/risk: slow operations or recovery if the journal grows greatly.
- Rollback need: none.

#### B. Minimal local improvement

- Change: instrument event count/replay duration and emit a non-authoritative
  threshold warning; retain full replay.
- Benefit: establishes the inversion threshold without adding authority state.
- Remaining problem: no latency bound after the threshold.
- Migration valley: telemetry/schema and operator-dashboard work; temporarily
  more operational signals without automated relief.
- Rollback: remove the warning; event bytes remain unchanged.

#### C. Cross-boundary structural change

- Change: introduce a content-bound, versioned checkpoint plus suffix replay,
  updated schema inventory, recovery tooling, and operator migration.
- Preconditions/owners: allocator, schema, host, deployment and operations must
  agree on checkpoint validity and atomic publication.
- Steady benefit: bounded normal replay and faster reconstruction.
- New cost/coupling: two persisted representations and checkpoint lifecycle.
- Migration valley: dual validation of full replay versus checkpoint+suffix;
  packaging and recovery become temporarily slower and more complex.
- Rollback: retain all events and ignore checkpoints; therefore feasible if
  compaction never deletes authoritative events.

### Score and verdict

- `E` externalization: 1 (future host/client latency).
- `A` change amplification: 1 (checkpoint would touch several repository areas).
- `F` boundary failure: 0 (none observed).
- `K` KPI divergence: 0 (current correctness and system outcome align).
- `T` time lock-in: 1 (migration remains easy while events are retained).
- `Severity`: **3/15**.
- `Confidence`: **C2** for linear work and tested replay; the actual inversion
  threshold remains a C0 hypothesis.
- Classification: `time-delayed`, low severity; no implementation change now.

## 6. Cross-cutting compensation structure

The only deliberate compensation is `ambiguous_prior_effect` at the protocol
acknowledgement seam. It burdens operators after a narrow crash window, but
prevents duplicate effects and does not create another allocation authority.

## 7. Rejected false positives

| Target | Initial signal | Rejection reason | Rationality |
|---|---|---|---|
| explicit expiry event | more caller/controller work than wall-clock expiry | wall-clock inference makes identical bytes replay differently | intentional audit/determinism redundancy |
| separate allocator journal | two durable stores | their authorities differ and the protocol fails closed between them | fault and authority separation |
| repeated protocol validation during replay | duplicate computation | it detects configuration/semantic inconsistency instead of duplicating the rule | defense with one canonical rule owner |

## 8. Unverified items and next evidence

| Priority | Evidence | Uncertainty resolved | Method |
|---:|---|---|---|
| 1 | replay p50/p95 by event count | actual inversion threshold | benchmark 10³–10⁶ lifecycle events on supported filesystems |
| 2 | crash/repair operator traces | burden of ambiguous acknowledgement | production/pilot incident log |
| 3 | filesystem matrix | hard-link and directory-sync portability | integration tests on supported deployment filesystems |

No new material finding emerged; implementation changes are not justified by
the evidence currently available.

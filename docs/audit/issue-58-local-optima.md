# Issue 58 implementation local-optima audit

## 1. Executive summary

- Mode: `intervention` after the first executable pilot passed.
- Scope: `scripts/runtime-integration-pilots.py`, its topology fixtures,
  `tests/runtime_pilots.rs`, the operational-host reconciliation seam, and the
  checked-in pilot evidence.
- System outcome: obtain credible real-runtime evidence without allowing a
  runtime declaration, a successful process, or a generated report to become
  accepted CaseGraphen state.
- Result: two high-confidence local shortcuts were removed. The file-drop
  worker now creates its own native report and commit inside a real Git
  worktree, and checked-in evidence is bound to harness/topology/host hashes.
  One cross-boundary constraint remains explicit: the operational host cannot
  yet supply resource expectations to `reconcile_with_resources`, so a
  resource-bearing run correctly stops incomplete.
- Evidence constraint: this is local subprocess evidence, not remote-runtime,
  sustained-load, crash-recovery, or organizational cost evidence.

## 2. System outcome and evaluation conditions

| Variable | Initial condition | Expanded condition used by the audit |
|---|---|---|
| `B` | one adapter and its generated JSONL | subprocess -> native boundary -> MCP host -> canonical reconciler -> operator review |
| `M` | test passes and expected halt code | provenance strength, resource safety, reproducibility, review integrity, future adapter cost |
| `N` | edit the pilot fixture only | edit harness, fixtures, executable test, evidence, and documentation; do not duplicate canonical CaseGraphen rules |
| `T` | one local run | repeated release evidence and v0-to-next-version decision |

Constraints retained: experimental v0 may change; the runtime is untrusted;
the pilot cannot mutate a case; resource decisions must not be inferred from
compact report metadata; shared files modified by other issue agents are not
rewritten for this issue.

## 3. Evidence used

| Observation plane | Evidence | Scope | Constraint |
|---|---|---|---|
| Structure | `scripts/runtime-integration-pilots.py:63-548`, `src/bin/casegraphen-mcp-host.rs:138-148`, `src/runtime_integration.rs:280-378` | report construction, host join, resource seam | static ownership does not prove production behavior |
| Execution | `cargo test --test runtime_pilots -- --nocapture`; checked-in `docs/pilots/issue-58/pilot-report.json` | two subprocess adapters and five scenarios | one machine and small inputs |
| Evolution | 107-commit Git signal scan; mean 6.28 and p90 15.4 files/commit; schema/example co-change pairs | repository-wide change amplification | pilot files are new, so no pilot-specific history exists |
| Meaning / organization | `accepted:false`, `unreviewed`, promotion evidence/unknown/blocker split | trust and release ownership | no team/SLO telemetry was available |

Observations above are facts. Cost ownership and future release drift below are
inferences unless explicitly backed by the executable pilot.

## 4. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Synthetic file-drop/worktree evidence | very small fixture | reviewer believes an adapter/worktree boundary that was not actually exercised | runtime boundary | 10 | C3 | mixed; fixed |
| 2 | Host `reconcile_run` always uses empty resource expectations | compact payload and one canonical path | resource-bearing adapters can never reach review through this host tool | product workflow | 10 | C3 | externalization; explicit blocker |
| 3 | Checked-in pilot snapshot without source binding | easy human inspection | later evidence can look current after harness/topology/host changes | release lifecycle | 7 | C2 | time-delayed; fixed |

## 5. Detailed candidate cards

### C1 — Synthetic file-drop/worktree evidence

**Facts.** In the first implementation, the subprocess wrote only
`shared.txt`; the harness then fabricated the native report, and `worktree_id`
named ordinary temporary directories. The revised implementation initializes a
temporary Git repository, creates two `git worktree` allocations, and each
runtime subprocess writes, commits, obtains its commit SHA, and drops its own
native report. The adapter reads those immutable native reports and artifact
bytes. The executable assertion proves two registered worktrees and distinct
commits.

**Local rationality.** Plain directories and harness-created reports made a
fast deterministic fixture. The pilot author benefited through low code and
setup cost.

**Compensation halo.** A reviewer or later runtime integrator would have had to
manually distinguish “directory named worktree” from an actual Git allocation,
and trust a report made on the consumer side of the boundary. That burden
falls on release reviewers every time the evidence is reused.

| Boundary | Initial approach | Cross-boundary alternative | Advantage |
|---|---|---|---|
| function | less setup | invoke Git and runtime-owned report creation | initial approach |
| module | fewer failure modes | verifies actual native report ingestion | alternative |
| workflow | cannot demonstrate worktree/commit lineage | real worktree + distinct commit evidence | alternative |
| lifecycle | misleading fixture can become canonical folklore | executable provenance remains inspectable | alternative |

Counterfactuals: A) retain synthetic directories (low effort, weak evidence);
B) rename claims to isolated directories (honest but does not meet worktree
pilot scope); C) use temporary real Git worktrees and runtime-authored native
reports (chosen; modest setup and cleanup cost). Rollback is removal of only the
temporary repository. `E=2, A=1, F=2, K=3, T=2`, severity 10, confidence C3,
classification `mixed` before the fix.

### C2 — Empty resource-expectation host seam

**Facts.** The operational host calls `GenericJsonlReconciler::reconcile`
(`src/bin/casegraphen-mcp-host.rs:138-148`). That method delegates to
`reconcile_with_resources(..., &[])` (`src/runtime_integration.rs:280-296`).
The canonical reconciler rejects topology resource claims without independent
expectations (`src/runtime_integration.rs:364-378`). The real file-drop pilot
therefore halts `resource_reconciliation_incomplete`, emits no proposal, and
keeps `accepted:false`.

**Local rationality.** A string topology plus string JSONL payload keeps the MCP
host thin and prevents it from inventing grants. The host implementer benefits
from a small decision surface and correct fail-closed behavior.

**Compensation halo.** Operators of any real resource-bearing topology must use
a custom Rust caller or wait at the host seam even when declarations,
reservations, and allocations exist. The compact tool API externalizes the
join work to every host integrator.

| Boundary | Current host | Resource-capable host input | Advantage |
|---|---|---|---|
| function | minimal and fail-closed | more parsing and joins | current host |
| module | canonical logic remains single-sourced | still delegates to the same canonical logic | tie |
| product workflow | cannot complete resource-bearing run | carries independently granted typed expectations | alternative |
| lifecycle | every adapter needs a side channel/custom caller | one versioned cross-contract input | alternative if stabilized |

Counterfactuals: A) keep the blocker (safe, operationally incomplete); B) add a
separate host tool that content-addresses typed expectations (smaller migration,
extra tool); C) extend `reconcile_run` with a versioned expectation bundle and
delegate to `reconcile_with_resources` (coherent steady state, schema and
compatibility migration). This issue records the blocker rather than changing
the concurrently implemented host contract. `E=3, A=2, F=2, K=2, T=1`,
severity 10, confidence C3, classification `externalization`.

### C3 — Unbound checked-in evidence snapshot

**Facts.** The run report is generated and checked in for review, while measured
latency and toolchain versions legitimately vary. Before intervention there was
no direct proof naming the harness bytes, topology fixture bytes, or host binary
that generated that snapshot. The report now includes SHA-256 values for all
three inputs plus Python and Git versions under `execution_provenance`; the
integration test regenerates evidence and checks protocol invariants rather
than exact latency bytes.

**Local rationality.** A standalone JSON snapshot is easy to read and avoids
platform-dependent golden failures. The immediate reviewer benefits.

**Compensation halo.** Future release reviewers otherwise need to reconstruct
whether the snapshot predates a behavior change. That cost repeats at every
promotion decision.

| Boundary | Snapshot only | Source-bound snapshot + executable assertions | Advantage |
|---|---|---|---|
| file | smallest JSON | more provenance fields | snapshot only |
| workflow | manual freshness check | direct byte binding | alternative |
| lifecycle | stale evidence can persist silently | changed sources visibly break the binding | alternative |

Counterfactuals: A) snapshot only; B) remove checked-in evidence and always rerun
(fresh but hurts offline review); C) keep a source-bound observation and rerun
semantic assertions (chosen). `E=2, A=1, F=1, K=1, T=2`, severity 7,
confidence C2, classification `time-delayed` before the fix.

## 6. Cross-cutting compensation halo

The common risk was “evidence-shaped metadata” replacing an observed boundary:
directory names replacing worktrees, consumer-created reports replacing
runtime-created reports, and snapshots replacing provenance. The intervention
makes the expensive boundary observable while retaining one canonical
reconciler. It intentionally does not make runtime identity, model, context,
cost, latency, commit, or deployment claims authoritative.

## 7. Designs not classified as local optima

| Design | Initial signal | Rejection reason | Rationality |
|---|---|---|---|
| Both adapters normalize to generic JSONL | shared adapter could look artificially uniform | source runtimes are materially different and sharing the canonical boundary avoids duplicated completeness rules | intentional bounded context |
| All proposals remain unreviewed and `accepted:false` | appears to block automation | the product outcome is trustworthy review, and the separation prevents runtime self-acceptance | security/audit constraint |
| Runtime cost/version/latency are declarations | values are caller-constructible | fields are explicitly marked untrusted and the promotion report lists this unknown | accurate observation strength |

## 8. Remaining unknowns and next evidence

| Priority | Evidence | Uncertainty resolved | Acquisition |
|---:|---|---|---|
| 1 | resource expectation bundle through the operational host | whether a resource-bearing run reaches review without a custom Rust caller | versioned host pilot with declaration/reservation/allocation fixtures |
| 2 | remote runtime with disconnect/replay | whether stdio/durable replay handles real runtime interruption | supervised integration pilot |
| 3 | repeated medium/large DAG runs | memory, JSONL payload, p95/p99, retry/cost behavior | load pilot with fixed budgets |
| 4 | fresh reviewer disposition of redesign proposal | whether content-addressed mismatch evidence supports a useful graph change | review through the dedicated topology review seam |

## 9. Intervention constraints and quality checklist

- Change scope: pilot-only files and a new integration test; no alternate
  readiness, resource, completeness, or review implementation.
- Migration: none for product contracts; the pilot now requires local Git.
- Temporary degradation: slightly slower test and more subprocess setup.
- Rollback: remove the temporary pilot workspace; no case state is written.
- Verified: local benefit explained; `B/M/N/T` explicit; burden owners named;
  structural and execution evidence combined; facts/inference separated;
  inversion tables and A/B/C counterfactuals included; severity and confidence
  separate; intentional trust boundaries considered as false-positive guards.

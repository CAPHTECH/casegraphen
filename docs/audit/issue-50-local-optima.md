# Issue 50 implementation local-optima audit

## 1. Executive summary

- Scope: experimental resource declaration, reservation, disposition,
  capacity, runtime allocation, reconciliation, and git-worktree records.
- System outcome: make topology resource claims enforceable by an external
  allocator without putting a scheduler, lock service, secret store, or
  destructive worktree cleanup in CaseGraphen.
- Conclusion: no high-severity local optimum remains. The leading candidate is
  the pure snapshot-based grant evaluator: it is deterministic and reusable,
  but atomic serialization remains the adapter's responsibility. This is an
  explicit protocol/service boundary, not a claim of process-wide locking.
- Evidence limit: static code, schemas, and deterministic tests are available;
  no production contention trace, adapter implementation, Git history trend,
  or operational ownership evidence exists.

## 2. Evaluation conditions

| Variable | Current condition | Expanded condition |
|---|---|---|
| `B` boundary | pure Rust protocol module and fixtures | allocator process, persistent state, runtime, git host, operator recovery |
| `M` metric | deterministic compatibility and exact reconciliation | race freedom, crash recovery, operational latency, cleanup safety |
| `N` change scope | experimental types/schemas/tests | adapter transaction boundary, store, worktree commands, control plane |
| `T` time | one v0 evaluation/reconciliation | concurrent grants, crashes, repeated cleanup, schema evolution |

The current design optimizes a narrow but intentional metric: one semantic
owner for compatibility and reconciliation while excluding runtime mutation.

## 3. Evidence

| Plane | Evidence | Observation | Constraint |
|---|---|---|---|
| Structural | `src/resource_protocol.rs`, seven schemas | Claims, grants, allocations, disposition/capacity, and reconciliation are distinct typed records; time is absent from release logic. | Static evidence. |
| Execution | `tests/resource_protocol.rs` | Ten tests observed reader compatibility, writer exclusion, rate capacity, identity joins, mismatch incompleteness, explicit release, scope rejection, and worktree findings. | In-process deterministic tests only. |
| Evolution | exact declaration-to-grant conversion and schema examples | Contract duplication is checked through serde/schema tests; no co-change history exists yet. | No longitudinal evidence. |
| Meaning/organization | ADR 0017 and design document | Liveness assertions belong to operators/adapters; CaseGraphen does not infer death or perform cleanup. | Adapter owner is not identified. |

## 4. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Snapshot-based grant evaluation | Pure deterministic function; no scheduler/store coupling | Adapter must serialize read-check-record atomically | two concurrent allocator calls | 7 | C2 | `externalization` |
| 2 | Mirrored claim/grant fields | Each fact is independently auditable | Schema additions require coordinated updates | repeated contract evolution | 4 | C1 | `time-delayed` |

## 5. Candidate LO-50-1: snapshot-based grant evaluation

### Facts, inference, and hypothesis

- **Observed:** `grant_reservation` receives existing reservations, explicit
  assertions, and capacities, and returns a grant/refusal without I/O.
- **Observed:** the design document explicitly says the caller must serialize
  competing grants atomically; elapsed time cannot alter active state.
- **Observed:** tests prove sequential conflicting grants are refused, but do
  not run two allocator processes.
- **Inference:** semantic compatibility is centralized, while transactional
  mutual exclusion is externalized to the allocator boundary.
- **Hypothesis:** an adapter that reads one stale snapshot in two processes can
  record two individually valid exclusive grants unless its own store provides
  compare-and-append/locking. No adapter exists here to reproduce that failure.

### Local rationality and compensation halo

- Local goal: portable, deterministic protocol logic with no persistence or
  scheduling dependency.
- Beneficiaries: runtime-adapter authors and protocol reviewers.
- Valid constraint: CaseGraphen intentionally is not the runtime scheduler.

| Local decision | Boundary effect | Compensation | Cost bearer | Evidence |
|---|---|---|---|---|
| Pure evaluation over supplied state | no cross-process atomicity | adapter transaction/lock around evaluate-and-record | adapter owner | documented; no adapter trace |
| Explicit assertion only | crashed reservation remains active | operator establishes death and appends release/supersede | operator | ADR 0017 plus old-timestamp test |

### Boundary inversion

| Boundary | Current approach | Integrated allocator alternative | Advantage |
|---|---|---|---|---|
| Function/module | deterministic, easy to test | persistence and lock coupling | current |
| Single serialized adapter | one semantic owner plus adapter transaction | duplicated service concerns in core | current |
| Multiple uncoordinated callers | stale-snapshot race possible | atomic allocator service | alternative |
| Lifecycle/operations | explicit stranded reservations are loud | automatic liveness risks false release | current safety trade-off |

- Minimum inversion boundary: multiple callers without one serialized state
  owner.
- Inverting metric: global mutual exclusion rather than local deterministic
  correctness.
- Time axis: the first real multi-process adapter deployment.

### Counterfactuals

- **A — current:** pure evaluator plus explicit adapter atomicity requirement.
  Lowest coupling; misuse is possible if the documented boundary is ignored.
- **B — local improvement:** hide a process-local mutex in the library. This
  looks convenient but does not protect multiple processes/hosts and creates a
  misleading safety claim; reject.
- **C — boundary-spanning change:** a persistent allocator/control-plane
  adapter executes evaluate-and-append under a compare-and-set revision or
  owned lock. It adds availability, recovery, persistence, and migration costs;
  rollback keeps records readable and returns allocation ownership to a single
  adapter. Validate this when #52/control-plane work chooses a store.

Scores: `E=2`, `A=1`, `F=2`, `K=1`, `T=1`, **Severity 7/15**,
**Confidence C2**. Verdict: `externalization`, bounded and explicitly exposed.
It is not high-severity within the protocol-only scope; representing a local
mutex as global enforcement would be the worse local optimum.

## 6. Candidate LO-50-2: mirrored claim/grant fields

- **Observed:** declaration uses topology `ResourceClaim`; grant/allocation
  repeat resource mode, group, workspace, network, and secret scopes.
- **Inference:** repetition is required to preserve declared/granted/actual
  facts independently, but increases coordinated schema changes.
- **Hypothesis:** frequent v0 field additions could produce change
  amplification. No history supports frequency yet.

Counterfactual A keeps explicit records; B shares more Rust helper types but
cannot eliminate wire fields; C collapses records and loses auditability.
The advantage does not invert at the current system boundary. Scores:
`E=0`, `A=2`, `F=0`, `K=0`, `T=2`, **Severity 4/15**, **Confidence C1**;
verdict `time-delayed` weak signal.

## 7. Rejected candidates and unverified gaps

| Target | Signal | Rejection reason |
|---|---|---|
| No real git worktree creation in tests | adapter appears incomplete | Avoiding destructive filesystem behavior is the requested reference-fixture boundary; records still expose base, path, branch, commit, dirty writes, and recoverable cleanup. |
| Stranded reservation after crash | unattended operation cost | This is a deliberate ADR 0017 safety trade: explicit recovery is louder than false time-based release. |
| Secret scopes in records | possible leakage | Reconciliation/grant validation accepts canonical named scopes and rejects assignment-shaped values; values remain outside the protocol. Broader secret scanners are untested. |

Next evidence:

1. Exercise the protocol through the first real adapter under concurrent grant
   attempts and observe its atomic state boundary.
2. Fault the adapter between grant persistence and runtime dispatch to validate
   explicit recovery and supersession.
3. Run actual safe worktree creation in a disposable repository at the adapter
   layer, including dirty/unexpected paths and recoverable cleanup.
4. Measure schema co-change after several revisions before treating mirrored
   fields as harmful amplification.

## 8. Cross-issue topology-binding correction

独立したdeclaration recordはauditabilityとruntime-neutralityに合理性がある一方、
従来のprotocol-level grant evaluatorは、そのdeclarationが実際のtopology id/hash/
node/claim setから作られたかを検査しなかった。`B`を一recordからdeployment graph、
`M`を内部整合からclaim substitution防止、`N`を#43 topologyと#50 grantへ、`T`を
grant前まで広げると、整合した偽declarationをadapterが補償検査する必要があった。

反実仮想Aはcallerを信頼、Bは各adapterがjoinを検査、Cはcontent-addressed
topology-aware grant entry pointを追加して既存compatibility evaluatorへ委譲する
案で、Cを採用した。stale hash、unknown node、substituted claimの三反例を同じ
APIで拒否する。構造証拠とfixture実行が一致するためconfidence `C2`、
`E=3,A=2,F=3,K=3,T=1`でseverity `12/15`、判定`externalization`（修正済み）。
allocatorのread-check-record atomicityはこのpure APIでは解決せず、既存の明示的
external boundaryとして残る。

## 9. Post-implementation audit: disposable Git worktree adapter

The adapter follow-up changes the evidence boundary that section 7 called out:
`src/worktree_adapter.rs` now creates, observes, and explicitly disposes real
isolated worktrees, while `tests/worktree_adapter.rs` exercises only uniquely
named repositories below the process temporary directory.

| Variable | Adapter condition | Expanded condition |
|---|---|---|
| `B` | one explicit repository/worktree/reservation tuple | hostile caller paths, concurrent allocator, Git/process crash |
| `M` | exact base/identity join, dirty/unexpected-write detection, recoverable removal | durable transactionality and unattended fleet recovery |
| `N` | adapter API plus disposable integration fixture | allocator store, operator authorization service, hosted Git policy |
| `T` | create → commit/observe → explicit dispose | interruption at each Git command and long-lived abandoned branches |

### Evidence and boundary inversion

| Plane | Evidence | Observation | Limit |
|---|---|---|---|
| Structural | `GitWorktreeRequest`, `GitWorktreeRecord`, disposition assertion | caller must supply absolute paths, exact 40-character base, branch, reservation/attempt, and allowed writes | OS-level path authorization remains the caller's responsibility |
| Execution | four disposable-repository integration tests | two attempts produce distinct branches/commits; dirty and committed unexpected writes are visible; mismatched assertion cannot remove | no forced process termination during `git worktree add/remove` |
| Evolution | existing resource record reused | adapter does not introduce a second resource-compatibility rule | request has no promoted wire schema yet |
| Meaning/organization | explicit release/supersede only | time never changes ownership; branch+commit remain after worktree removal | operator authority is represented, not externally authenticated |

The first implementation exposed three high-confidence local-optimum
candidates during review:

1. **Descriptive fixtures instead of executable isolation (`12/15`, C3,
   externalization).** The pure record layer optimized safety by leaving all Git
   behavior to an unspecified adapter. It inverted as soon as the boundary was
   an actual runtime integration. Fixed with a disposable-repository adapter
   test and explicit API.
2. **Dirty-only inspection (`11/15`, C3, time-delayed).** Looking only at the
   current status makes a clean committed write invisible. Fixed by unioning
   base-to-HEAD, unstaged, staged, and untracked paths before applying the
   allowed-write boundary.
3. **Convenient cleanup (`13/15`, C3, downstream-shift).** Removing on timeout
   or with force would simplify local lifecycle handling but could erase an
   active actor's only uncommitted state. The adapter instead requires an exact
   disposition assertion, refuses dirty/unexpected writes, runs non-forced
   removal, and retains the branch/result commit.

Counterfactual A kept record-only fixtures and left correctness unobserved.
Counterfactual B added automatic timeout cleanup, improving apparent liveness
while weakening resource safety. Counterfactual C, selected here, spans the
record/adapter boundary but preserves the core decision rule: reservation
compatibility remains in `resource_protocol`, while filesystem mutation is an
explicit external action. Rollback is deletion of the adapter surface; all v0
records remain readable.

Residual candidate: `git worktree add -b` and record persistence are not one
durable transaction (`E=2,A=1,F=2,K=2,T=2`, severity `9/15`, C2,
`externalization`). A crash can leave a branch or registered worktree before
the caller persists the returned record. Solving this locally would create a
misleading process-only transaction. The minimum inversion boundary is the
future allocator store, which must journal intent and reconcile Git's
registered worktree list after restart. This is a bounded residual, not a claim
that the adapter provides crash-atomic allocation.

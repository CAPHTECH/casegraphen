# Worker Execution Security And Approval Policy

Status: Accepted 2026-07-30. This is the Phase 5 gate required before
effectful workers may be enabled against a real project. The legacy
implementation deferred effectful workers precisely because they "can execute
external processes, write files outside CaseGraphen state, or use network
credentials"; this document is the deliberate pass that deferral called for.

## 1. Threat model

Actors and trust levels:

| Actor | Trust |
|---|---|
| Human reviewer (plan accept, review commands) | Trusted within named capabilities |
| CaseGraphen CLI itself | Trusted; the deterministic control plane |
| LLM / agent proposing plans, morphisms, completions | Untrusted candidate generator |
| Worker process output (stdout/stderr, side effects) | Untrusted until validated |
| Files on disk between commands (plans, bindings, logs) | Tamper-evident, not tamper-proof |

Primary risks:

1. A worker (or an LLM-authored binding) executes something broader than what
   the reviewer approved.
2. Worker output is promoted to accepted fact without validation.
3. State is mutated outside the morphism log (bypassing replay/audit).
4. A stored plan or binding is edited after acceptance.
5. Secrets leak into worker environments or durable evidence.

## 2. Controls in effect

### 2.1 Execution is disabled by default

`run --step` and `run --frontier` refuse shell bindings unless
`--enable-worker shell` is passed explicitly on every invocation. There is no
configuration file that enables workers persistently; enabling is a
per-invocation, auditable decision.

### 2.2 Capability and policy gates

Every durable native case-state mutation covered by the CLI requires an operation gate:
named actor, non-empty capability ids, scope bound to the case space,
audit/system audience, and a source-boundary match. The enforced operation
strings are:

| Command | Operation |
|---|---|
| `plan accept` / `plan reject` | `plan-review` |
| `run --step` / `run --frontier` | `dispatch` |
| `morphism apply` | `morphism-apply` |
| `morphism reject` | `morphism-reject` |
| `evidence attach` | `evidence-attach` |
| `cell transition` | `cell-transition` |
| `review accept\|reject\|reopen\|waive` | `review` |

`morphism propose` is intentionally ungated because it writes only a proposal
file and does not mutate durable case state. Applying or rejecting that
proposal is gated.

Every capability id must resolve to an active/accepted
`custom:capability` cell with accepted provenance, and that cell's
`metadata.actor_ids` must name the acting actor. Capability cells are a
source-boundary trust root: they may enter a case space only in the
materialized genesis supplied at lift/import time. The shared morphism reducer
rejects generic addition, update, or retirement of capability cells, and
`cell transition` cannot change them. There is no post-genesis CLI capability
administration path.

The same shared reducer also prevents generic updates from changing any
cell's `cell_type`. For evidence cells it additionally makes the entire
`provenance` object and metadata keys `evidence_boundary`, `content_hash`,
`trace_id`, and `worker_report_id` immutable. Evidence promotion remains a
canonical review morphism, not an evidence-cell rewrite.

For `morphism apply`, `morphism reject`, `evidence attach`, `cell transition`,
and all four `review` actions, the validated gate is stored as
`morphism.metadata.operation_gate`. Both run modes use the same actor for their
dispatch gate and appended log entries. `run --frontier` validates one dispatch
gate for the invocation; its capability ids must cover every binding selected
for the round.

Capability ↔ OS permission mapping (operator duty, not enforced by the tool):

| Declared capability | Operator must ensure |
|---|---|
| `capability:worker-exec:<scope>` | The OS user running `casegraphen` has no broader filesystem access than the binding's `working_directory` subtree requires |
| `capability:worker-net:<scope>` | Only bindings whose commands genuinely need network run under an environment with credentials; otherwise run offline |
| `capability:store-write:<scope>` | The case store directory is writable only by the operating user |

The tool cannot verify OS sandboxing; it records who claimed what. Running
workers under a dedicated OS user or container is the operator's control.

### 2.3 Pre-authorization is narrow and content-addressed

- A plan is executable only after an explicit accept with reviewer id and
  reason, recorded as a morphism in the hash-chained log.
- The acceptance records `plan_content_hash`; both `run --step` and
  `run --frontier` re-verify the stored plan (review_status normalized) against
  that hash. Editing a plan after acceptance is detected and refused.
- Binding content hashes are recorded into the plan at propose time
  (`plan.metadata.worker_binding_hashes`); editing a binding after acceptance
  yields a `binding_hash_mismatch` obstruction and no dispatch.
- Binding registration stores the canonical command and working-directory
  paths plus the command file's SHA-256. Dispatch re-resolves and re-hashes
  them before spawning from the pinned canonical paths; symlink retargeting
  yields `binding_identity_mismatch`.
- Auto-application of a worker-driven transition happens only when the
  transition falls inside the plan's `allowed_transition_classes`
  (morphism type × cell types × lifecycles). Anything outside is stored as an
  unreviewed proposal and surfaces as `transition_not_authorized`.

### 2.4 Worker containment

- Environment is cleared; only `env_allowlist` variables pass through.
  `PATH`, every `LD_*`/`DYLD_*` loader namespace variable, and the reserved
  `CASEGRAPHEN_*` namespace are rejected even if listed.
- `command` and `working_directory` must be absolute. Both are canonicalized
  immediately before spawn; the command must resolve to an executable file (a
  file carrying no execute bit is refused before spawning, so "could not be
  executed" is classified the same way on every host rather than depending on
  whether `setsid` is present) and the working directory must resolve to a
  directory.
- Timeout is mandatory. On Unix, when absolute `setsid` and `kill` utilities
  are available, the worker is launched in a dedicated session and timeout
  kills the process group. Otherwise the direct child is killed and the worker
  report records `descendants_may_survive: true`.
- Stdout/stderr reader waits are bounded by a two-second grace. Captured output
  records whether the stream was incomplete, so descendants holding a pipe
  cannot block dispatch indefinitely.
- Stdout/stderr retain at most 4 MiB, but their SHA-256 and `byte_len` cover
  the complete stream whenever `incomplete` is false.
- Worker exit codes and timeouts are domain findings (evidence +
  obstructions), never silent.

### 2.5 Output trust boundary

Worker output enters the space only as evidence cells with
`review_status: unreviewed` provenance and recorded content hashes, under the
trust boundary marker
`local_process_output_untrusted_until_validated_and_reviewed`. Inferred or
worker-produced material never satisfies a hard evidence requirement until
review promotes it or a deterministic check validates it. "The command
succeeded" (CommandOutput evidence) and "the goal is achieved" (requirement
satisfied + invariants pass) remain distinct judgments.

### 2.6 Audit

Every selected step in either run mode writes its own `execution_trace`
(plan/step/binding hashes, worker report id, appended entry ids, obstructions,
information loss) under `runs/`, and appends that trace's content hash to the
hash-chained morphism log. Selection-time binding hash, identity, and
per-binding capability refusals are failed traces too; they are not transient
report-only findings. Run directories are atomically reserved and a `started`
trace exists before worker spawn, so concurrent dispatch cannot reuse an
in-progress attempt and failures after reservation retain a trace.

`run --frontier` dispatches at most `--max-parallel` workers concurrently,
then applies and anchors their results serially in plan-step order. One
validated dispatch gate covers the round, but traces, attempts, obstructions,
and anchors remain per-step. A per-step dispatch, reservation, or application
failure is reported in the round result and does not suppress the report for
other selected steps. A stale `started` trace can be superseded only by an
explicit `--retry-step` after its recorded reserved base revision is no longer
current; a `started` trace at the current revision remains protected as a
concurrent dispatch.

Replay wins over any cache or snapshot. The audit path for an incident is:
trace → worker report + raw output hashes → log entries → revision replay.

## 3. Approval policy — what always needs a human

| Action | Human review required? |
|---|---|
| Plan acceptance | Always (reviewer id + reason + gate) |
| Plan rejection | Always (reviewer id + reason + gate) |
| Binding registration | No, but its hash is frozen into any plan that uses it, and plan review is the checkpoint |
| Generic morphism proposal | No; it is an ungated, non-durable proposal-file write |
| Generic morphism apply or reject | Always (reviewer id + reason + operation gate) |
| Evidence attachment | No promotion; the attachment requires an operation gate and enters as unreviewed evidence |
| Direct cell transition | Capability-gated; human interaction is not required when a delegated actor holds the imported capability |
| Transition inside accepted plan classes, deterministic gates pass | No additional review (the accepted plan plus dispatch gate authorizes it) |
| Transition outside plan classes | Always — remains an unreviewed proposal |
| Review accept/reject/reopen/waive | Always (reviewer id + reason + operation gate) |
| Promoting worker evidence to satisfy a hard requirement beyond `source_backed` origin rules | Always (`review accept`, with operation gate) |
| Case-space close | Always (close-check invariants incl. gate) |
| Enabling a new worker kind (beyond `shell`) | New design review; extend this document first |

## 4. Residual risks (accepted)

1. The tool does not sandbox workers at the OS level; containment relies on
   the operator-run environment (§2.2). Mitigation: run under a dedicated
   user/container; keep `--enable-worker` off in shared shells.
2. Hash chains detect tampering but cannot prevent a writer with store access
   from rewriting the whole log; the store directory must be access-controlled
   and, for high-assurance use, backed up append-only.
3. `env_allowlist` review remains a human judgment beyond the built-in loader,
   path, and reserved-namespace deny-list; a reviewer can still approve another
   secret-bearing variable.
4. Hosts without usable `setsid` and `kill` utilities cannot guarantee
   descendant termination. The direct child is killed, reader waits remain
   bounded, and `descendants_may_survive` makes that residual risk explicit.
5. Pinning a binding's executable identity proves nothing about an
   interpreter's arguments: a binding whose command is `/bin/sh` with a `-c`
   script, or any interpreter reading a script file, is fixed only in its
   interpreter hash. Reviewers must read the full `args` during plan review and
   prefer bindings that name a single pinned program.
6. A narrow window remains between the dispatch-time canonicalization of the
   command and working directory and the spawn itself; an attacker who can
   rewrite those paths in that window can still substitute code. Store and
   working directories must be writable only by the operating user.
7. Capability grants are fixed at lift/import time and cannot be revoked
   through the CLI. Revoking a grant means lifting a new case space (or
   migrating), which is deliberate: revocation is a source-boundary decision,
   not a runtime one.
8. An operator who can write to the store can hold or repeatedly recreate a
   case lock and deny writes. An empty pre-created run directory is not skipped:
   it hard-errors that step's reservation. `run --frontier` records and reports
   that per-step failure while continuing the round when it can reserve a
   separate failure trace; `run --step` returns the reservation error. These are
   availability, not integrity, risks and are bounded by store filesystem
   permissions.

## 5. Review provenance

This document is not an assertion of safety by construction. Its claims were
checked against the implementation by three adversarial review rounds, each of
which found real defects that were then fixed:

- Round 1 found ten defects (four critical), including snapshot-only state
  tampering that replay accepted and case creation that replaced an existing
  history. Both were reproduced on a real store before the fix and confirmed
  blocked after it.
- Round 2 judged four of those fully fixed and six partial, and found a new
  critical defect: reusing an earlier revision id overwrote that revision's
  snapshot, so a refused operation still left the store permanently invalid.
  Reproduced and confirmed fixed.
- Round 3 found that the capability gate was only syntactic: an attacker could
  replace an accepted capability cell through the generic morphism path and
  grant themselves the capability the gate checks, because that path — like
  every other durable mutation except plan review and dispatch — was ungated.
  Reproduced and confirmed fixed by §2.2.

Anyone extending the execution surface should assume the same treatment is
required: the controls here hold only for the paths that were actually attacked.

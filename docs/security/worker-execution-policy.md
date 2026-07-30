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

`run --step` refuses shell bindings unless `--enable-worker shell` is passed
explicitly on every invocation. There is no configuration file that enables
workers persistently; enabling is a per-invocation, auditable decision.

### 2.2 Capability and policy gates

Every durable native case-state mutation covered by the CLI requires an operation gate:
named actor, non-empty capability ids, scope bound to the case space,
audit/system audience, and a source-boundary match. The enforced operation
strings are:

| Command | Operation |
|---|---|
| `plan accept` / `plan reject` | `plan-review` |
| `run --step` | `dispatch` |
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
`morphism.metadata.operation_gate`. `run --step` uses the same actor for its
dispatch gate and appended log entries.

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
- The acceptance records `plan_content_hash`; `run --step` re-verifies the
  stored plan (review_status normalized) against that hash. Editing a plan
  after acceptance is detected and refused.
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
  immediately before spawn; the command must resolve to a file and the working
  directory must resolve to a directory.
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

Every step writes an `execution_trace` (plan/step/binding hashes, worker
report id, appended entry ids, obstructions, information loss) under `runs/`,
and appends that trace's content hash to the hash-chained morphism log. Run
directories are atomically reserved and a `started` trace exists before worker
spawn, so concurrent dispatch cannot reuse an in-progress attempt and failures
after reservation retain a trace. Replay wins over any cache or snapshot. The
audit path for an incident is:
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

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

Every dispatch requires an operation gate (`check_operation_gate`, operation
`dispatch`): named actor, non-empty capability ids, scope bound to the case
space, audit/system audience, and a source-boundary match. Plan acceptance
requires the same gate with operation `plan-review`. Capability ids named in a
gate MUST correspond to grants recorded as cells or policy metadata in the
case space; a gate that names capabilities nobody recorded is reviewable in
the log and attributable to its actor.

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
- Auto-application of a worker-driven transition happens only when the
  transition falls inside the plan's `allowed_transition_classes`
  (morphism type × cell types × lifecycles). Anything outside is stored as an
  unreviewed proposal and surfaces as `transition_not_authorized`.

### 2.4 Worker containment

- Environment is cleared; only `env_allowlist` variables pass through. No
  allowlist entry may name known secret-bearing variables unless the reviewer
  accepted that binding knowingly — reviewers MUST treat `env_allowlist` as
  the secret-exposure surface during plan review.
- `working_directory` is fixed by the reviewed binding; timeout is mandatory;
  stdout/stderr are captured with a hard cap and hashed.
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
and every state change is an entry in the hash-chained morphism log. Replay
wins over any cache or snapshot. The audit path for an incident is:
trace → worker report + raw output hashes → log entries → revision replay.

## 3. Approval policy — what always needs a human

| Action | Human review required? |
|---|---|
| Plan acceptance | Always (reviewer id + reason + gate) |
| Binding registration | No, but its hash is frozen into any plan that uses it, and plan review is the checkpoint |
| Transition inside accepted plan classes, deterministic gates pass | No (this is exactly what plan acceptance authorized) |
| Transition outside plan classes | Always — remains an unreviewed proposal |
| Promoting worker evidence to satisfy a hard requirement beyond `source_backed` origin rules | Always (`review accept`) |
| Case-space close | Always (close-check invariants incl. gate) |
| Enabling a new worker kind (beyond `shell`) | New design review; extend this document first |

## 4. Residual risks (accepted)

1. The tool does not sandbox workers at the OS level; containment relies on
   the operator-run environment (§2.2). Mitigation: run under a dedicated
   user/container; keep `--enable-worker` off in shared shells.
2. Hash chains detect tampering but cannot prevent a writer with store access
   from rewriting the whole log; the store directory must be access-controlled
   and, for high-assurance use, backed up append-only.
3. `env_allowlist` review is a human judgment; a reviewer can still approve a
   secret-leaking binding. Mitigation: reviewers check allowlists against a
   deny-list of known credential variables.

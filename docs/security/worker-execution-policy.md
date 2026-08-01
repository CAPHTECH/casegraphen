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
| Files on disk between commands (plans, bindings, logs) | Tamper-evident except at the log tail — see residual risk 2 |

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
`custom:capability` cell with accepted provenance, that cell's
`metadata.actor_ids` must name the acting actor, and its `metadata.operations`
must list the operation being performed (ADR 0007). The last of those is what
makes a role split enforced rather than descriptive: without it, any capability
an actor held admitted every gated operation, and the walkthrough's dispatch-only
runner could promote evidence with its dispatch capability. There is no
permissive default — an absent or empty `operations` list authorizes nothing,
because a default meaning "every operation" is the separation an author just
modelled, silently undone. Capability cells are a
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

Immutability alone left the addition direction open, so the reducer also refuses
an *added* evidence cell that declares any `evidence_boundary` other than
`inferred` or `worker_output` outside the genesis entry. Those two are the only
spellings the tool mints after genesis — `evidence attach` forces the first and
`run --step` records the second — and `source_backed` is read as acceptable with
no review at all, so accepting it from a payload would have made a hard evidence
requirement satisfiable by typing a string. Genesis stays exempt: it is the
declared trust root where source-backed evidence legitimately enters.

Whether a piece of evidence *covers* a requirement is decided the same way:
from the log, not from the graph. The evaluator reads coverage only from the
morphisms that mint it — the genesis entry, which is the declared trust root,
and every `evidence_attach` morphism, whose payload records the `--satisfies`
targets that `evidence attach` and `run --step` checked before building it. A
`satisfies_evidence_requirement` edge added by a generic `morphism apply`, an
evidence cell's post-genesis `structure_ids`, and a relation's `evidence_ids`
are all still in the graph and are all still shown; none of them satisfies a
hard requirement. Before this, any actor the gate admitted could point already
promoted evidence at a requirement nobody reviewed it for, and the obstruction
disappeared with no review anywhere in the log.

A genesis coverage claim only counts for a target genesis itself materialized.
`structure_ids` is a free-form string list — the shipped example uses file
paths — so an entry naming an id nothing has created is not a coverage claim
about a future cell. Without that restriction a genesis author could reserve an
id, and the work created with it later would be born covered by trusted
evidence with no review naming it; that was reproduced against the shipped
walkthrough genesis, with a control run confirming the coverage claim was the
only difference.

Both keys the log is read by — `review` and `evidence_attach` — are therefore
reserved: `morphism propose` and `morphism apply` refuse a generic morphism that
declares either `morphism_type`, alongside the canonical review metadata keys
they already refused. A proposal file is written by the caller, so a type that
is read back as proof that a command ran is a caller-declared trust value unless
the tool is the only writer of it.

A relation update may not change `relation_type`, `from_id`, `to_id`, or
`relation_strength`. Those four are the identity of an edge, and leaving them
writable meant the coverage hardening could be walked around from the other end:
one gated update moved a hard `requires_evidence` edge onto an already-accepted
evidence cell and the blocking obstruction disappeared, with no review recorded
anywhere. Changing an edge is retire-plus-add, which the log shows as two
operations rather than one silent rewrite. Annotation — metadata, source ids,
provenance, evidence ids — stays mutable; the readiness decision does not read
it.

`append_morphism` validates the resulting case space against the loader's
contract — the store's reference check *and* the evaluator's
`validate_native_case_space` — before it writes anything. The second half was
missing until 2026-08-01, and the gap was the same shape as the one it was
meant to close: the writer checked a subset of what every read enforces. Three
ordinary gated commands reached it. `evidence attach` never inspects a cell's
`space_id` or `title`, so either one wrote a store where every derived command
failed permanently; and a `retire` of any relation dangles the log entries that
named it. All three wrote successfully, and `space validate` reported
`valid: true` on the result while `space rebuild` reported success, because the
fold verifies checksums rather than this contract.
Previously only the loader checked, so a gated `retire` of a relation that a
projection still named was written and then refused by every read path,
including the write paths that could have repaired it — and `space rebuild`, the
documented recovery, reported success on it because the fold verifies checksums
rather than that contract.

The evidence-boundary rule is enforced in both reducer entry points, so it also
runs during the fold that `space replay`, `space rebuild`, and `space validate`
perform. A case
space that already recorded such an entry therefore stops loading rather than
loading with the declared trust intact. That is the intended direction: no path
in this tool ever produced one — after genesis it mints only `inferred` and
`worker_output` — so a log carrying one records a trust claim the tool never
made, and reading it is the wrong default.

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
  immediately before spawn; the command must resolve to a file and the working
  directory to a directory. **On Unix** the command must additionally carry an
  execute bit, and a file without one is refused before spawning, so "could
  not be executed" is classified the same way on every Unix host rather than
  depending on whether `setsid` is present. There is no such check off Unix —
  `require_executable` is a no-op there — so on those hosts the refusal comes
  from the spawn instead, and this control is Unix-only. Everything in §2.4
  below is likewise Unix-only; see residual risk 10.
- Timeout is mandatory. On Unix, when absolute `setsid` and `kill` utilities
  are available, the worker is launched in a dedicated session and its process
  group is signalled on all four exit paths: clean exit, timeout, incomplete
  output, and poll failure. If the group `KILL` fails, a signal-zero probe
  distinguishes a still-visible group from one with no signalable members, and
  a probe that could not be spawned concludes nothing. Incomplete output never
  reuses an earlier conclusive outcome — it is positive evidence that something
  still holds the worker's pipe, so the group is re-signalled and re-probed.
  On timeout and poll failure the direct child is killed unconditionally: it is
  the one containment step that does not depend on the utilities being
  trustworthy.
- The launcher is resolved only from `/usr/bin/setsid` and `/bin/setsid`.
  `/usr/local/bin` and `/opt/homebrew/bin` were candidates and are not, because
  they are group-writable on a normal developer machine: the process actually
  spawned is `setsid <pinned command>`, so a launcher an approved worker could
  replace would defeat the command pinning it wraps. A host with no system
  `setsid` loses containment and says so.
- `descendants_may_survive: false` means the group was signalled, or a probe
  that ran reported no signalable members. It is not a proof that nothing
  survived: `kill(-pgid, SIGKILL)` succeeds when it reaches at least one
  member, and a member it may not signal is invisible to both calls (§4,
  residual risk 4). `true` means containment was not established, including
  every host without both utilities.
- Stdout/stderr reader waits are bounded by a two-second grace. Captured output
  records whether the stream was incomplete, so descendants holding a pipe
  cannot block dispatch indefinitely.
- Stdout/stderr retain at most 4 MiB, but their SHA-256 and `byte_len` cover
  the complete stream whenever `incomplete` is false.
- Worker exit codes and timeouts are domain findings (evidence +
  obstructions), never silent.
- One accepted step has one live dispatch. A `started` trace remains blocking
  across every graph revision and is released only by a named operator
  assertion, `--supersede-trace <trace-id>`, against that exact trace. The tool
  never infers process liveness from revision movement, process identity, or a
  timeout.

### 2.5 Output trust boundary

The evidence cell carries `output_incomplete` and `output_truncated` beside
its `content_hash`, because the hash means "the whole stream" only when the
stream was whole — a reader seeing the hash alone could not tell it covered a
prefix. Both are frozen on update alongside the hash they qualify.

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
other selected steps — **except when another process wins the same step**: a
round whose worker ran and then lost the append exits non-zero with no report
at all, discarding the results of steps that had already completed. Measured
under contention, not hypothetical. The traces stay coherent and nothing is
double-executed, so this is a reporting gap rather than an integrity one, but
the sentence above is not true under contention and the round's own work is
what goes unreported. `--retry-step` applies only to failed traces. A `started`
trace can be superseded only when `--supersede-trace` names that exact trace;
the superseding trace records the asserted id under
`metadata.superseded_trace_ids`, covered by the anchored trace content hash. A
later dispatch for the same step remains protected even if an operator repeats
an older assertion.

Replay wins over any cache or snapshot. The log's constant-size head file is an
independent witness for the current tail entry; a missing or stale head refuses
the audit path rather than silently trusting the log. The audit path for an
incident is: trace → worker report + raw output hashes → anchored log entries →
revision replay. A trace is anchored when its dispatch finishes, not when it
starts, because its content includes the entry ids and result revision the
dispatch has not produced yet — so between the evidence append and the
transition append the evidence morphism is in the hash-chained log while the
trace explaining it is still unverified. The fact of the evidence is
tamper-evident there; the explanation is not. `metadata.worker_invoked` is
written into the trace file at spawn rather than only at finish, so a
dispatcher killed mid-round leaves a record that says which reserved steps
had a real process — the question `--supersede-trace` asks an operator to
answer.

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
| Adopting an existing morphism log (`space rebuild --adopt-existing-log`) | Always — the operator asserts that the pre-existing unanchored log is trusted; rebuild verifies its full fold and snapshots before creating a missing head |
| Enabling a new worker kind (beyond `shell`) | New design review; extend this document first |

## 4. Residual risks (accepted)

1. The tool does not sandbox workers at the OS level; containment relies on
   the operator-run environment (§2.2). Mitigation: run under a dedicated
   user/container; keep `--enable-worker` off in shared shells.
2. Hash chains detect tampering but cannot prevent a writer with store access
   from rewriting the log, and **a rollback of the tail is not detectable at
   all**. Measured, not assumed: rewriting one middle entry is refused
   (`log entry ... previous_entry_hash`), tampering with a snapshot alone is
   refused (`snapshot checksum`), and truncating the log is refused by the head —
   but deleting the last entry *and* rewriting the head leaves a store that
   `space validate` reports as `valid: true`, with the erased decision gone and
   no warning anywhere. Forging that head needs no computation: the entry being
   deleted carries the required `entry_hash` in its own
   `previous_entry_hash` field.

   One head/log disagreement is *not* tampering and must not be treated as it:
   the log append and the head write are two operations, so a crash between
   them — Ctrl-C is enough — leaves the head naming an earlier entry of an
   otherwise intact log. That state used to refuse every command including
   `space rebuild --adopt-existing-log`, the documented recovery, which left
   deleting the head by hand as the only way out — the exact primitive this
   risk calls an untraceable rollback, and indistinguishable from one
   afterwards. `space rebuild --adopt-existing-log` now repairs a head that
   verifies as an earlier entry of the log and rewrites it to the tail. A head
   *ahead* of the log, or naming a present revision with a different checksum,
   is the rollback or rewrite signature and still refuses, on every path.

   The rest is structural. The head is the only anchor for the tail and it
   lives in the same directory, writable by the same principal, so no in-store
   mechanism can distinguish a rollback from a store that simply has fewer
   revisions.
   Detecting it requires an anchor the tool does not write: commit the store
   directory to version control, or record `current_revision_id` and the head's
   `replay_checksum` wherever the decision they represent is acted on, and
   compare before trusting a later read. The store directory must be
   access-controlled and, for high-assurance use, backed up append-only.

   The worked recipe (issue #15 decided this stays an operator recipe — the
   tool gets no read-side assertion flag, because a refusal that only some
   read commands honoured would be a decision rule in one place and not its
   sibling, this project's recurring defect). When you act on a decision,
   record the store's head next to wherever the decision lands (a PR
   description, a ticket, a deploy record):

   ```sh
   cat "$STORE/native_case_spaces/<escaped case space id>/morphism_log.head.json"
   # {"target_revision_id": "...", "entry_hash": "...", "replay_checksum": "sha256:..."}
   ```

   Before trusting a later read, compare two strings against
   `space inspect --format json` output: the anchored `target_revision_id`
   must still appear under `result.record.revisions[]`, and that entry's
   `replay_checksum` must equal the anchored one. The anchored revision
   missing from the list is a rollback; the same revision id with a different
   checksum is a rewrite. Either finding means the store no longer contains
   the history the decision was made against — stop and investigate before
   appending anything.
3. `env_allowlist` review remains a human judgment beyond the built-in loader,
   path, and reserved-namespace deny-list; a reviewer can still approve another
   secret-bearing variable.
4. Hosts without usable `setsid` and `kill` utilities cannot guarantee
   descendant termination. The direct child is killed, reader waits remain
   bounded, and `descendants_may_survive` makes that residual risk explicit.

   Three limits remain with both utilities present, and none of them is
   detected:

   - **A member the tool may not signal.** `kill(-pgid, SIGKILL)` returns
     success when it reached at least one member, so a group holding one
     ordinary process and one unsignalable one (a setuid boundary, say)
     reports containment. The signal-zero probe is not run after a successful
     `KILL`, deliberately: on the paths where the child is not yet reaped it
     would see our own exiting group and report survivors on every healthy
     run, making the field permanently `true`. Signalled is therefore what
     `false` claims, not empty.
   - **A `kill` that fails for a reason other than an absent group.** The
     probe's verdict is a process exit status, so `EPERM` and a usage error
     read the same as "no such process". A `kill` implementation that rejects
     the `--` separator would make every run on that host conclude an empty
     group having measured nothing.
   - **A recycled process group id.** The group is signalled with the direct
     child's pid, which the reaping `try_wait` has already released. POSIX
     keeps a pgid reserved while the group has members, so this cannot produce
     a false `false` — but on a pid-churning host the tool may deliver
     `SIGKILL` to an unrelated group that recycled the number.
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
   case lock and deny writes. Lock contention is not only adversarial: the
   lock is held across the evaluator's contract check, the snapshot, the
   append and the head write, so hold time scales with the case space — 3.0 s
   measured for one gated `cell transition` on a 4,000-cell space. The wait
   budget is a 30 s deadline, chosen to outlast an ordinary append and to stay
   well under the 60 s staleness threshold; a case space large enough to make
   a single append exceed it will contend with itself, and the fixed budget is
   a limit rather than a guarantee. An empty pre-created run directory is not skipped:
   it hard-errors that step's reservation. `run --frontier` records and reports
   that per-step failure while continuing the round when it can reserve a
   separate failure trace; `run --step` returns the reservation error. These are
   availability, not integrity, risks and are bounded by store filesystem
   permissions.
9. **A worker chooses how much a store keeps, and pruning it is one-way.**
   The 4 MiB cap bounds what is retained *in memory* and in the evidence cell;
   `runs/<trace>/stdout` and `stderr` get every byte the worker wrote. Anchor
   verification hashes all three artifacts of every anchored trace on each
   dispatch — streamed since 2026-08-01, so memory is constant, but the CPU
   and the disk are proportional to what workers have produced, cumulatively
   and permanently. The cost that survives the streaming fix is **time**:
   hashing is CPU-bound whether or not it is buffered, measured at 4.4 s for a
   `run --step` that dispatched nothing after one worker wrote 100 MB, paid on
   every dispatch from then on. There is no retention or GC path: truncating a stream to
   reclaim space makes the anchor verification refuse from then on, which is
   correct — the artifact no longer matches what was anchored — but leaves no
   supported way to prune. Operators running large-output workers should cap
   them at the worker, and treat a store's `runs/` directory as append-only.
10. **Worker execution is a Unix control surface.** Off Unix, `require_executable`
   and `process_group_utilities` are both stubs: there is no execute-bit check
   before the spawn, the worker is never launched into a dedicated session, and
   `descendants_may_survive` is unconditionally `true`. The crate compiles for a
   non-Unix target (checked against `wasm32-unknown-unknown`) and the rest of
   the tool — the gate, the reducer, evidence trust, the store contract — is
   platform independent, so this bounds dispatch only. No non-Unix host has been
   exercised end to end; the claims in §2.3 and §2.4 should be read as Unix
   claims until one is.

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

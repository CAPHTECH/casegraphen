---
name: casegraphen-operate
description: Use when driving work through a CaseGraphen case space with the casegraphen CLI — lifting a case space, reading readiness or blockers, proposing and applying a morphism, attaching or promoting evidence, transitioning a cell, registering a worker binding, accepting a plan, running a step, or governing an agent runtime's graph by recording what it produced as reviewable evidence. Covers the revision and gate discipline every mutating command needs and the refusals that will otherwise waste attempts.
---

# Operating a case space

You propose; CaseGraphen decides. Nothing you assert about trust is accepted:
content hashes, evidence boundaries, resolved paths, and review status are
computed or forced by the tool. Plan for refusals — they are the interface, not
errors to work around.

For a worked example of every command below with its real output, read the
[release-decision walkthrough](https://github.com/CAPHTECH/casegraphen/blob/main/docs/guides/release-decision-walkthrough.md).
The checked-in command, flag, status, halt, and refusal vocabulary is generated
from the shipped CLI surface and schemas in
[`capabilities.generated.md`](capabilities.generated.md); CI refuses drift.

## Route

| Task | Read |
|---|---|
| Create a case space, or model goals/work/evidence so readiness comes out right | `references/authoring.md` |
| Change the graph: add, update, retire cells or relations; attach evidence and the artifacts it cites; promote evidence; transition a cell; drive attach → review → transition from one packet | `references/mutating.md` |
| Have a worker do the work: binding, plan, `run --step`, reading the result | `references/executing.md` |
| An agent runtime executes the graph and CaseGraphen records what was accepted: node granularity, mandates, taking runtime reports as evidence | `references/governing.md` |

For a reviewed execution topology, accept the exact topology/policy artifact
through `topology-review` first. An operational host may then invoke
`compile_reviewed_deployment_bundle` using the returned revision and claim
cell; the host derives authority from the store. This does not accept the
generated execution plan or any runtime result.

## The two rules that break every first attempt

**1. Carry the revision returned by each mutating command.** Each durable
mutation creates a new revision, and a stale `--base-revision-id` is refused,
never merged. Take the next base revision from
`result.record.current_revision_id` in the response of the command that just
wrote. `run --step` alone appends up to three entries; its response carries the
revision after all of them.

At the first command of a session, or after any refusal or failure, recover by
re-reading instead:

```sh
cur() {
  casegraphen space inspect --store "$STORE" --case-space-id "$CS" --format json \
    --output inspect-report.json >/dev/null &&
    python3 -c 'import json;print(json.load(open("inspect-report.json"))["result"]["record"]["current_revision_id"])'
}
REV="$(cur)"
```

Pass `--base-revision-id "$REV"`. After each successful durable mutation, set
`REV` from that command's response; do not call `cur()` between successful
mutations. The same applies inside a morphism proposal: its `source_revision_id`
must equal `$REV` at apply time, so write the proposal file immediately before
applying it.

**Read `--format json` refusals as JSON, not prose.** Every refusal — not only
a successful report — honors `--format json` and is written to stderr as
`{"error_code": ..., "message": ..., "data": {...}}`, never to stdout or
`--output`. A stale-revision refusal (`"error_code": "stale_revision"`) hands
back the current revision directly in `data.current_revision_id`: read that
instead of calling `cur()` to recover. `error_code` also distinguishes what
kind of retry makes sense:

- `"usage"` and `"invalid"` mean fix the call and do not retry as-is —
  including the specific case of a gate flag missing entirely
  (`--capability-id`, `--actor-id`, ...): that is a pre-flight completeness
  check, not a gate decision, so it classifies as `"usage"` even though the
  message is about a gate.
- `"gate_violation"` means the gate was actually evaluated and refused: a
  different actor or capability is required, not a different call shape with
  the same identity. **On `plan accept`/`plan reject` specifically, treat
  this as escalate to a human reviewer holding the right capability, not
  retry** — plan review is the one operation
  `docs/security/worker-execution-policy.md` marks always-human, and this is
  that checkpoint refusing.
- `"stale_revision"` means re-read `data.current_revision_id` and retry the
  same call with it.
- `"stale_plan_revision"` looks similar but is not the same recovery: the
  *plan* was built against a case space that has since moved, so regenerate
  the plan against `data.current_revision_id` — retrying the same plan file
  with a corrected `--base-revision-id` will not help, since the plan's own
  content is what is stale.
- `"missing_case_space"` / `"store_integrity"` mean stop and check `--store`
  before retrying at all — `store_integrity` also covers a plan whose stored
  review no longer matches the operation gate it was accepted under, not
  only a store replay mismatch.
- `"lock_unavailable"` means another process holds this case space's
  exclusive lock and **retrying will not resolve it**. The tool never infers
  that a live lock is abandoned (ADR 0017), so there is no staleness timeout
  that eventually reclaims it, and each attempt costs the full 30 s wait
  budget. Stop and escalate: someone must establish externally whether the
  holder is still running. Removing the `.lock` file named in the refusal
  message is the assertion that it is gone — a human act, in the same class as
  `--supersede-trace`, and wrong if the holder is merely slow. Nothing durable
  landed; the store is exactly where it was.
- `"io_error"` means the command may already have completed a durable
  mutation and then failed on something unrelated (a bad `--output` path,
  a JSON re-render failure) — **do not blindly retry**, since the mutation
  may have already landed. Check `completed_through` on the refusal first;
  if absent, re-read the current revision before deciding whether the
  original call is still needed at all.

**A refusal can carry `completed_through` at the top level** (a sibling of
`error_code`, not inside `data`): the `current_revision_id` after the last
durable mutation the command completed, when the command already knows it —
e.g. an append succeeded and only a later `--output` path or JSON re-render
failed afterward. Present only when the command that refused already held
that revision; absent otherwise, including when the mutation this refusal
itself blocked never landed. When present, use it the same way as
`data.current_revision_id` on a `stale_revision` refusal — as `$REV` for the
next call — instead of calling `cur()`.

Every command accepts `--output <path>`. It writes the full JSON report there
and emits nothing on stdout — measured 0 bytes. For anything but the smallest
report, use `--output`, then extract only the field you need:

```sh
REV="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["result"]["record"]["current_revision_id"])' write-report.json)"
# or: REV="$(jq -r '.result.record.current_revision_id' write-report.json)"
```

| Question | Narrow read command |
|---|---|
| What can proceed? | `space frontier` |
| What blocks? | `obstruction list` |
| What is the terminal status view? | `space reason --format text` |
| What is the full folded state? | `space replay --output <path>` |
| Where is it and how far has it advanced? | `space inspect` — revisions, paths, counts |

`space inspect` answers *where and how far*: revisions, log and snapshot paths,
counts. It carries no cells, so it cannot tell you what the space contains. The
folded state comes from `space replay`, and `space frontier` / `space reason`
derive from it — do not read an empty `inspect` payload as an empty space. The
text reason view renders the same evaluator result as JSON; use it for a human
terminal read, not as a second decision rule.

`space reason` reports two independent status axes, and you must read both:
`Progress` (`active | blocked | complete`) says whether the work is moving or
done; `Assurance` (`unreviewed | review_required | accepted | rejected`) says
whether what it produced is trusted. `Progress: complete` with
`Assurance: review_required` is the normal shape of finished-but-unreviewed
work — it means run reviews, not stop.

The text view's other sections are read-only projections of the same
evaluation the JSON payload carries — nothing here is a separate decision:

- **Waiting** — `readiness.waiting_cell_ids`, the cells a hard dependency is
  currently blocking.
- **Unaccepted evidence findings** — an unaccepted finding that names evidence
  whose review gap the evaluator has already marked satisfied (the
  requirement-placeholder pattern in `authoring.md`) prints
  `[requirement_satisfied=true]` next to it. Read both facts together: the
  evidence itself is still unaccepted, *and* the hard requirement it would
  satisfy is already covered another way. Neither line contradicts the other.
- **Review gaps** — every open review obligation
  (`review_gaps`), each with its own `requirement_satisfied`. A gap can be
  open (not yet reviewed) and simultaneously not blocking Assurance
  (`requirement_satisfied=true`) — that combination is expected, not a bug.
- **Changed since** — only present with `--since-revision <revision-id>`,
  covered next.

This view never shows *why work is stopped* (no halt/reason section): a halt
is a property of a plan plus its traces (`run`/`operate`'s own reports), and
`space reason` takes no `--plan-id` — approximating one from obstructions here
would be exactly the re-derivation this view is built to avoid. Read
`run`/`packet apply`/`operate`'s own `result.halt` for that.

`--since-revision <revision-id>` adds a "Changed since" section listing the
log entries recorded after that revision — an assertion, not a lookup: the
revision must already be in this case space's history (the same discipline as
`--base-revision-id`), and an unknown one is refused rather than resolved to
"nearest". It only means something for the text render, so it is refused
combined with `--format json`.

```sh
casegraphen space reason --store "$STORE" --case-space-id "$CS" --format text --since-revision "$REV"
```

**2. Every durable mutation needs a valid operation gate.** Five resolved values
are checked against the case space. Put reusable values in a strict named JSON
profile and let the reference files below pass its argv selection as `$GATE`:

```sh
GATE_PROFILE_FILE="<path-to-operation-gate-profiles.json>"
GATE="--gate-profile <name> --gate-profile-file $GATE_PROFILE_FILE"
```

`$GATE` is written unquoted at the call sites, so the shell splits it into
separate arguments. If you build argv yourself rather than through a shell, pass
the selection flags as separate elements. The profile document has schema
`highergraphen.case.operation_gate_profiles.v1`, schema version `1`, and a
`profiles` array. A complete entry looks like this:

```json
{
  "name": "operator-audit",
  "actor_id": "actor:operator",
  "capability_ids": ["capability:durable-mutation"],
  "operation_scope_id": "case_space:example",
  "audience": "audit",
  "source_boundary_id": "source_boundary:example"
}
```

A profile may be partial. Add any remaining gate flags at the call site; an
explicit `--actor-id`, `--capability-id`, `--operation-scope-id`, `--audience`,
or `--source-boundary-id` overrides that profile field. Missing values are still
refused, and every expanded value goes through the same case-space checks. The
log records the expanded gate, never the profile name. Profiles cannot supply
`--enable-worker`, `--base-revision-id`, or reviewer identity.

| Requirement | How it fails |
|---|---|
| `--operation-scope-id` equals the case-space id | `violates: operation_scope_id` |
| `--audience` is `audit` or `system` | `violates: audience` |
| `--source-boundary-id` equals the boundary declared in `metadata.source_boundary.id` | `violates: source_boundary_id` |
| each `--capability-id` resolves to an accepted, active `custom:capability` cell | `does not resolve to an existing case cell` |
| that cell's `metadata.actor_ids` contains `--actor-id` | `does not grant acting actor` |
| that cell's `metadata.operations` lists the operation below | `does not authorize operation <op>` |

The operation string is chosen by the command, not by you:
`plan accept`/`plan reject` → `plan-review`, `run --step` → `dispatch`,
`morphism apply` → `morphism-apply`, `morphism reject` → `morphism-reject`,
`evidence attach` → `evidence-attach`, `cell transition` → `cell-transition`,
all four `review` actions → `review`.

`morphism propose` is the only ungated write: it produces a proposal file and
mutates nothing.

## Reading results

Exit code 0 with obstructions in the payload is the normal shape of bad news.
Stale revisions, integrity mismatches, and invalid `--supersede-trace`
assertions are tool failures.

That default is unchanged. For CI, pass `--strict` to a finding-carrying read or
run command: exit 2 means the graph says no, while exit 1 means the tool broke;
the report payload is unchanged.

- `status: step_failed` with `worker_execution_failed` — the command ran and
  reported failure. Evidence was still attached; no transition was applied.
- `status: no_dispatchable_step` with `retry_required` — a previous attempt
  failed. Retrying is an explicit act: pass `--retry-step <step-id>`.
- `dispatch_in_progress` — a `started` trace still blocks the step. Only after
  externally establishing that exact dispatch is dead, pass
  `--supersede-trace <trace-id>`; revision movement never releases it.
- `transition_not_authorized` — the transition fell outside the accepted plan's
  `allowed_transition_classes` and was kept as an unreviewed proposal.
- `binding_hash_mismatch` / `binding_identity_mismatch` — the plan or the
  binding's command changed after acceptance. Nothing ran. Do not re-register to
  make it pass; that discards the review.
- `status: paused_for_review` from `packet apply` — not an error. The attach
  landed; the packet stops there by design. Carry `completed_through` and read
  `next_operations`.
- `packet resume` refuses until **another actor** has accepted the claim, and
  refuses a claim that is not the evidence the named `--completed-through`
  revision attached. A stored `review_status: accepted` on the cell does not
  count; only a review morphism does.

## Never do these

- **Do not edit a snapshot, a log entry, a stored plan, or a registered
  binding.** All four are hash-verified; the store will refuse to load or
  dispatch, and repairing that costs more than redoing the operation.
- **Do not claim `review_status: accepted` on input.** `evidence attach` refuses
  it outright, and other paths force the boundary to an untrusted value. Use
  `review accept` to promote.
- **Do not try to create or amend a `custom:capability` cell.** It is refused at
  `morphism propose`. Capabilities enter only in the genesis materialization; a
  grant change means lifting a new case space.
- **Do not author a `custom:artifact` cell, an `artifact:sha256-…` id, or any
  change to an existing artifact.** That namespace is minted only by
  `evidence attach --artifact`, from a file the tool hashed. Authoring one is
  refused at propose and at lift, updating, transitioning, or retiring one is
  refused, and `review accept` on one is refused — review the claim that cites
  it instead.
- **Do not relax a check to make a run pass.** A refusal means the model or the
  input is wrong.

## Verify what you did

```sh
casegraphen space validate --store "$STORE" --case-space-id "$CS" --format json --output validate-report.json   # log fold reproduces the snapshot
casegraphen space history  --store "$STORE" --case-space-id "$CS" --format json --output history-report.json    # actor + operation per entry
casegraphen obstruction list --store "$STORE" --case-space-id "$CS" --format json --output obstruction-report.json # derived blockers now
```

Report what the obstructions say, not what you intended. A case space whose
blockers you have not read is a case space you have not advanced.

`space history --format text` reads more easily than the JSON when a step was
superseded and retried: a dispatch believed dead (`--supersede-trace`, ADR
0014) never gets its own log entry, so the *surviving* entry's line grows a
`(N attempts: <superseded-trace-id>, ..., <this-trace-id>)` annotation instead
of leaving the superseded id undiscoverable. That annotation appears only when
the surviving trace's own file names the superseded ids — a merely adjacent
entry for the same step with nothing naming it stays two separate lines, and
if a trace file cannot be read at all, every entry renders unfolded (with a
note saying so) rather than guessing. It is a read: the log itself is
unchanged either way, and `space history --format json` remains the source of
truth for the entries themselves.

`space validate` proves the log reproduces the snapshot; it cannot prove the
tail was not rolled back, because the head lives in the store. When a decision
leaves the store — a PR, a ticket, a deploy — copy the anchored pair from the
mutating response (`current_revision_id`) and the store's
`morphism_log.head.json` (`replay_checksum`) into that record. Before trusting
a later read, check the anchored revision still appears in `space inspect`
revisions with the same checksum; missing means rollback, changed means
rewrite — stop and investigate.

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

## Route

| Task | Read |
|---|---|
| Create a case space, or model goals/work/evidence so readiness comes out right | `references/authoring.md` |
| Change the graph: add, update, retire cells or relations; attach or promote evidence; transition a cell | `references/mutating.md` |
| Have a worker do the work: binding, plan, `run --step`, reading the result | `references/executing.md` |
| An agent runtime executes the graph and CaseGraphen records what was accepted: node granularity, mandates, taking runtime reports as evidence | `references/governing.md` |

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

`space validate` proves the log reproduces the snapshot; it cannot prove the
tail was not rolled back, because the head lives in the store. When a decision
leaves the store — a PR, a ticket, a deploy — copy the anchored pair from the
mutating response (`current_revision_id`) and the store's
`morphism_log.head.json` (`replay_checksum`) into that record. Before trusting
a later read, check the anchored revision still appears in `space inspect`
revisions with the same checksum; missing means rollback, changed means
rewrite — stop and investigate.

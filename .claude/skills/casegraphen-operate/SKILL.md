---
name: casegraphen-operate
description: Use when driving work through a CaseGraphen case space with the casegraphen CLI — lifting a case space, reading readiness or blockers, proposing and applying a morphism, attaching or promoting evidence, transitioning a cell, registering a worker binding, accepting a plan, or running a step. Covers the revision and gate discipline every mutating command needs and the refusals that will otherwise waste attempts.
---

# Operating a case space

You propose; CaseGraphen decides. Nothing you assert about trust is accepted:
content hashes, evidence boundaries, resolved paths, and review status are
computed or forced by the tool. Plan for refusals — they are the interface, not
errors to work around.

For a worked example of every command below with its real output, read
`docs/guides/release-decision-walkthrough.md`.

## Route

| Task | Read |
|---|---|
| Create a case space, or model goals/work/evidence so readiness comes out right | `references/authoring.md` |
| Change the graph: add, update, retire cells or relations; attach or promote evidence; transition a cell | `references/mutating.md` |
| Have a worker do the work: binding, plan, `run --step`, reading the result | `references/executing.md` |

## The two rules that break every first attempt

**1. Re-read the revision before every mutating command.** Each durable
mutation creates a new revision, and a stale `--base-revision-id` is refused,
never merged. `run --step` alone appends up to three entries. So:

```sh
cur() { casegraphen space inspect --store "$STORE" --case-space-id "$CS" --format json |
          python3 -c 'import json,sys;print(json.load(sys.stdin)["result"]["record"]["current_revision_id"])'; }
```

and pass `--base-revision-id "$(cur)"` every time. The same applies inside a
morphism proposal: its `source_revision_id` must equal the current revision at
apply time, so write the proposal file immediately before applying it.

**2. Every durable mutation needs a valid operation gate.** Five flags, and all
five are checked against the case space:

```sh
--actor-id <id> --capability-id <id> [--capability-id <id>…] \
--operation-scope-id "$CS" --audience audit --source-boundary-id <declared boundary id>
```

| Requirement | How it fails |
|---|---|
| `--operation-scope-id` equals the case-space id | `violates: operation_scope_id` |
| `--audience` is `audit` or `system` | `violates: audience` |
| `--source-boundary-id` equals the boundary declared in `metadata.source_boundary.id` | `violates: source_boundary_id` |
| each `--capability-id` resolves to an accepted, active `custom:capability` cell | `does not resolve to an existing case cell` |
| that cell's `metadata.actor_ids` contains `--actor-id` | `does not grant acting actor` |

The operation string is chosen by the command, not by you:
`plan accept`/`plan reject` → `plan-review`, `run --step` → `dispatch`,
`morphism apply` → `morphism-apply`, `morphism reject` → `morphism-reject`,
`evidence attach` → `evidence-attach`, `cell transition` → `cell-transition`,
all four `review` actions → `review`.

`morphism propose` is the only ungated write: it produces a proposal file and
mutates nothing.

## Reading results

Exit code 0 with obstructions in the payload is the normal shape of bad news.
Only a stale revision or an integrity mismatch is a tool failure.

- `status: step_failed` with `worker_execution_failed` — the command ran and
  reported failure. Evidence was still attached; no transition was applied.
- `status: no_dispatchable_step` with `retry_required` — a previous attempt
  failed. Retrying is an explicit act: pass `--retry-step <step-id>`.
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
casegraphen space validate --store "$STORE" --case-space-id "$CS" --format json   # log fold reproduces the snapshot
casegraphen space history  --store "$STORE" --case-space-id "$CS" --format json   # actor + operation per entry
casegraphen obstruction list --store "$STORE" --case-space-id "$CS" --format json # derived blockers now
```

Report what the obstructions say, not what you intended. A case space whose
blockers you have not read is a case space you have not advanced.

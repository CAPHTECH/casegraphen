# Walkthrough: deciding a release with CaseGraphen

A worked example that drives one case space from lift to a refused close. It
covers the whole control model — derived readiness, gated mutation, worker
dispatch, untrusted worker output, review promotion, runtime structure change,
and tamper detection — against the real binary rather than in prose.

Everything below was executed; the quoted output is from a single run against
0.8.0 on 2026-07-30. Hashes depend on file contents and paths, so yours will
differ. Two behaviours that surprised the author on that run are called out in
[What the run teaches](#what-the-run-teaches); read that section before copying
this shape into a real case space.

The case space asks a decision question: **may we tag v0.9.0?**

```
goal:release-0-9-0
├── work:schema-id-gate        every shipped schema carries an $id (a worker checks this)
└── work:tag-release           cut the tag — the irreversible step
        depends_on ─────────→  work:schema-id-gate          (hard)
        requires_evidence ──→  evidence:changelog-updated   (hard, unsatisfied placeholder)
capability:release-{plan-review,dispatch,worker-exec,durable-mutation}
```

`capability:*` cells are the authorization trust root and can only enter at
lift time. Two actors exist: `actor:release-manager` reviews, and
`actor:release-runner` dispatches. Neither can do the other's job.

## Setup

```sh
REPO=$(pwd)                                   # this repository
GUIDE="$REPO/docs/guides/release-decision"
WORK=$(mktemp -d)                             # store plus binding scripts
CG="$REPO/target/debug/casegraphen"
CS=case_space:casegraphen-release-0-9-0
SB=source_boundary:release-0-9-0-intent

cargo build
mkdir -p "$WORK/store"
cp "$GUIDE"/*.sh "$WORK/" && chmod +x "$WORK"/*.sh

# every gated mutation by the reviewing actor uses the same gate
GATE="--actor-id actor:release-manager --capability-id capability:release-durable-mutation \
      --operation-scope-id $CS --audience audit --source-boundary-id $SB"

# the current revision — every gated command needs it, and it moves constantly
cur() { "$CG" space inspect --store "$WORK/store" --case-space-id "$CS" --format json |
          python3 -c 'import json,sys;print(json.load(sys.stdin)["result"]["record"]["current_revision_id"])'; }
```

## 1. Lift the intent

[`release-decision/genesis.case.space.json`](release-decision/genesis.case.space.json)
is a genesis snapshot: its single log entry carries the full cell and relation
payload plus the immutable case-space shell, so the log alone reconstructs the
space.

```sh
"$CG" lift native --store "$WORK/store" --input "$GUIDE/genesis.case.space.json" \
  --revision-id revision:release-genesis --format json
```

```
current: revision:release-genesis | revisions: 1
```

The declared `revision.checksum` in the input is ignored and recomputed.

## 2. Read the derived state

```sh
"$CG" space frontier    --store "$WORK/store" --case-space-id "$CS" --format json
"$CG" obstruction list  --store "$WORK/store" --case-space-id "$CS" --format json
```

```
frontier: ['goal:release-0-9-0', 'work:schema-id-gate']
[high]   unresolved_dependency: work:tag-release depends on unresolved cell work:schema-id-gate.
[medium] missing_evidence:      work:tag-release requires source-backed or accepted evidence
                                evidence:changelog-updated, but none is available.
```

Nothing stores "tag-release is blocked". The frontier and both obstructions are
recomputed from the graph on every command.

## 3. Register the worker binding

[`release-decision/gate-schema-ids.sh`](release-decision/gate-schema-ids.sh) is
the gate. Note that `command` is the script itself, not `/bin/sh -c "…"`: that
pins the script's own content hash instead of the interpreter's. Also note the
script calls `/usr/bin/grep` by absolute path — the worker environment is
cleared and `PATH` may never be allowlisted.

```jsonc
{
  "binding_id": "worker_binding:schema-id-gate",
  "worker_kind": "shell",
  "command": "<WORK>/gate-schema-ids.sh",
  "args": [],
  "working_directory": "<REPO>",
  "resolved_command_path": "/caller/value/is/ignored",
  "resolved_working_directory": "/caller/value/is/ignored",
  "command_content_hash": "0000000000000000000000000000000000000000000000000000000000000000",
  "env_allowlist": [],
  "timeout_ms": 10000,
  "capability_ids": ["capability:release-worker-exec"]
}
```

```sh
"$CG" binding register --store "$WORK/store" --input "$WORK/gate.binding.json" --format json
```

```
resolved_command_path: <WORK>/gate-schema-ids.sh
command_content_hash: c9ada029636c17e8…
```

The caller-declared identity fields were discarded and replaced with measured
values. Declaring trust in the input never grants it.

## 4. Propose, check, and accept the plan

The plan names one step: run the gate against `work:schema-id-gate`, require
`evidence:schema-id-gate-clean`, and authorize exactly one transition class
(`update` of a `work` cell to `resolved`).

```sh
"$CG" plan propose --store "$WORK/store" --case-space-id "$CS" --input "$WORK/gate.plan.json" --format json
"$CG" plan check   --store "$WORK/store" --case-space-id "$CS" --plan-id plan:release-schema-id-gate --format json
```

```
plan_content_hash: 38b5f5f146becdab…
step_readiness: [{"on_readiness_frontier": true, "step_id": "step:run-schema-id-gate",
                  "work_cell_id": "work:schema-id-gate"}]
```

Accepting it with the dispatch actor instead of the reviewing actor fails:

```
operation gate for "plan-review" violates: capability capability:release-plan-review
does not grant acting actor actor:release-runner; metadata.actor_ids must contain the gate actor id
```

With the right actor:

```sh
"$CG" plan accept --store "$WORK/store" --case-space-id "$CS" --plan-id plan:release-schema-id-gate \
  --reviewer-id reviewer:release --reason "Deterministic gate, single pinned script, no network" \
  --base-revision-id "$(cur)" --actor-id actor:release-manager \
  --capability-id capability:release-plan-review --operation-scope-id "$CS" \
  --audience audit --source-boundary-id "$SB" --format json
```

```
accepted -> revision:plan-review:plan~3arelease-schema-id-gate:2
```

Acceptance records `plan_content_hash`, so editing the plan afterwards is
detected at dispatch.

## 5. Effectful workers are off by default

```sh
"$CG" run --step --store "$WORK/store" --case-space-id "$CS" \
  --plan-id plan:release-schema-id-gate --base-revision-id "$(cur)" \
  --actor-id actor:release-runner \
  --capability-id capability:release-dispatch --capability-id capability:release-worker-exec \
  --operation-scope-id "$CS" --audience audit --source-boundary-id "$SB" --format json
```

```
shell worker kind is disabled by default; pass --enable-worker shell
current revision after the refusal: revision:execution-trace-anchor:…:run-schema-id-gate~3a1
```

The refusal is not free. The run directory was already reserved and a trace was
anchored, so the revision moved and attempt 1 is spent. Adding
`--enable-worker shell` alone is now not enough:

```
status: no_dispatchable_step
obstruction: retry_required - step step:run-schema-id-gate has a failed execution trace;
             pass --retry-step step:run-schema-id-gate to retry it
```

A failed attempt never silently re-runs; retrying is an explicit decision.

## 6. Execute one step

```sh
"$CG" run --step --store "$WORK/store" --case-space-id "$CS" \
  --plan-id plan:release-schema-id-gate --base-revision-id "$(cur)" \
  --actor-id actor:release-runner --enable-worker shell \
  --retry-step step:run-schema-id-gate \
  --capability-id capability:release-dispatch --capability-id capability:release-worker-exec \
  --operation-scope-id "$CS" --audience audit --source-boundary-id "$SB" --format json
```

```
status: step_executed | dispatch: completed | transition: True
appended entries: 3            (evidence_attach, update, trace anchor)
unsatisfied: []
exit_status: 0 | stdout hash: cb5ffc590e1d254f…
```

What entered the case space:

```
evidence:worker-output:…:2   review: unreviewed   boundary: worker_output
relation: satisfies_evidence_requirement  diagnostic  -> evidence:schema-id-gate-clean
work:schema-id-gate lifecycle: resolved
```

The command succeeded, so the authorized transition was applied — but its output
entered as `unreviewed` `worker_output` evidence linked by a `diagnostic`
relation. "The command exited 0" and "the goal is achieved" stay separate
judgments. The raw stdout, its byte length, and its SHA-256 are retained under
`runs/<trace>/`.

## 7. Change the structure while the work is in flight

Suppose the contract-change checklist turns out to apply: 0.9.0 edits a shipped
schema, so tagging must wait for that review. That is a new cell and a new hard
dependency, added at runtime:

```sh
"$CG" morphism propose --store "$WORK/store" --case-space-id "$CS" \
  --input "$WORK/structure-change.morphism.json" --format json
"$CG" morphism check   --store "$WORK/store" --case-space-id "$CS" \
  --morphism-id morphism:add-contract-change-review --format json
```

```
proposal: checked
applicable: True
```

Applying against the revision the proposal was written for, after the run has
moved the store on, fails:

```
base revision revision:release-genesis is stale;
current revision is revision:execution-trace-anchor:…:run-schema-id-gate~3a2
```

Against the current revision it applies, and the derived frontier moves:

```
applied -> revision:add-contract-change-review
frontier: ['goal:release-0-9-0', 'work:contract-change-review']
```

The graph shape is fully mutable this way — cells and relations can be added,
updated, and retired, and `custom:<name>` cell and relation types extend the
vocabulary without a schema change. The authorization root is not:

```sh
# a morphism that adds actor:attacker to metadata.actor_ids of a capability cell
"$CG" morphism propose --store "$WORK/store" --case-space-id "$CS" \
  --input "$WORK/self-grant.morphism.json" --format json
```

```
morphism morphism:self-grant-capability cannot update capability cell
capability:release-durable-mutation: custom:capability cells are administered only
at lift/import time inside the declared source boundary
```

That is refused at `propose`, before any gate is even consulted.

## 8. Attaching evidence is not promoting it

The changelog evidence claims `source_backed` provenance and accepted review:

```
evidence attach input cell evidence:changelog-0-9-0-entry cannot claim accepted provenance;
use review accept to promote evidence
```

Downgrading the claim to `reviewed` lets it in, but not on its own terms:

```
stored boundary: attached_unverified | declared: source_backed
content_hash: 29f2def847bebfbd…
```

```sh
"$CG" obstruction list --store "$WORK/store" --case-space-id "$CS" --format json
```

```
missing_evidence     - work:tag-release requires source-backed or accepted evidence
                       evidence:changelog-updated, but none is available.
unresolved_dependency - work:tag-release depends on unresolved cell work:contract-change-review.
```

Attaching changed nothing about readiness. Promotion is a review morphism:

```sh
"$CG" review accept --store "$WORK/store" --case-space-id "$CS" \
  --target-id evidence:changelog-updated --reviewer-id reviewer:release \
  --reason "CHANGELOG 0.9.0 section verified against the attached document" \
  --base-revision-id "$(cur)" --evidence-id evidence:changelog-0-9-0-entry $GATE --format json
```

```
review -> revision:review:evidence~3achangelog-updated:9
obstructions: 1   (only the dependency remains)

evidence:changelog-updated provenance.review_status: unreviewed
```

The last line is the point: the cell was **not** edited. Promotion lives in the
review morphism, and the evaluator consults the log — so the audit trail records
who promoted what and why, and no code path can promote by rewriting a cell.

## 9. The lifecycle table is not advisory

```sh
"$CG" cell transition … --cell-id work:contract-change-review --to resolved $GATE
```

```
morphism morphism:cell-transition:… cannot transition cell work:contract-change-review
lifecycle from Proposed to Resolved
```

`proposed → active → resolved` works, and the frontier reaches the irreversible
step:

```
frontier: ['goal:release-0-9-0', 'work:tag-release']
```

## 10. A worker that reports failure

[`release-decision/tag-dry-run.sh`](release-decision/tag-dry-run.sh) checks that
`Cargo.toml` declares the version being tagged. It declares 0.8.0, so:

```
cli exit status: 0
status: step_failed | dispatch: failed | transition: False
unsatisfied: ['evidence:tag-dry-run-clean']
obstruction: worker_execution_failed - worker worker_binding:tag-dry-run exited with Some(1)

runs/…tag-dry-run~3a1/stdout:
tag-dry-run FAILED: Cargo.toml declares 0.8.0, tag would be v0.9.0
```

The CLI exits 0: a failing gate is a domain finding, not a tool error. Evidence
was attached anyway, and no transition was applied.

## 11. Fixing the gate instead of the code

The tempting move is to edit the pinned script:

```sh
sed -i '' 's/expected=0.9.0/expected=0.8.0/' "$WORK/tag-dry-run.sh"   # GNU sed: -i
"$WORK/tag-dry-run.sh"
```

```
tag-dry-run ok: Cargo.toml declares 0.8.0
```

Through CaseGraphen, with the same accepted plan:

```
status: no_dispatchable_step | worker ran: False
obstruction: binding_identity_mismatch
  worker binding worker_binding:tag-dry-run resolved identity no longer matches registration
  (command <WORK>/tag-dry-run.sh, working directory <REPO>, command hash bd4965f909303d2f…)
```

The edited script passes on its own and never runs here. Identity is re-measured
immediately before spawn and compared with what registration recorded.

## 12. Wiring a gate so it actually blocks

After the failure, the derived state was:

```
frontier: ['goal:release-0-9-0', 'work:tag-release']
obstructions: 0
```

The failed gate did not block tagging, because
`success_evidence_requirement_ids` is a **plan-level** condition: it gates
whether the step's transition may be applied, not whether the work is ready.
Readiness only knows about relations. Wiring the same requirement into the graph
fixes it:

```sh
# morphism_type: relate, adding
#   work:tag-release --requires_evidence(hard)--> evidence:tag-dry-run-clean
"$CG" morphism apply --store "$WORK/store" --case-space-id "$CS" \
  --morphism-id morphism:wire-dry-run-requirement --base-revision-id "$(cur)" \
  --reviewer-id reviewer:release --reason "Make the pre-tag gate a hard readiness requirement" \
  $GATE --format json
```

```
frontier: ['goal:release-0-9-0']
[medium] missing_evidence - work:tag-release requires source-backed or accepted evidence
                            evidence:tag-dry-run-clean, but none is available.
```

## 13. Audit

```sh
"$CG" space validate --store "$WORK/store" --case-space-id "$CS" --format json
"$CG" space rebuild  --store "$WORK/store" --case-space-id "$CS" --format json
"$CG" space history  --store "$WORK/store" --case-space-id "$CS" --format json
```

```
valid: True | entries: 17
rebuilt revisions: 17

 1  create                         actor:release-manager    (genesis)
 2  review                         actor:release-manager    plan-review
 3  custom:execution_trace_anchor  actor:release-runner     dispatch
 4  evidence_attach                actor:release-runner     dispatch
 5  update                         actor:release-runner     dispatch
 6  custom:execution_trace_anchor  actor:release-runner     dispatch
 7  create                         actor:release-manager    morphism-apply
 8  evidence_attach                actor:release-manager    evidence-attach
 9  review                         actor:release-manager    review
10  update                         actor:release-manager    cell-transition
11  update                         actor:release-manager    cell-transition
12  create                         actor:release-manager    morphism-apply
13  review                         actor:release-manager    plan-review
14  evidence_attach                actor:release-runner     dispatch
15  custom:execution_trace_anchor  actor:release-runner     dispatch
16  custom:execution_trace_anchor  actor:release-runner     dispatch
17  relate                         actor:release-manager    morphism-apply
```

Every durable entry after genesis names an actor and an enforced operation.
`space validate` proves a full fold of the log reproduces the snapshot;
`space rebuild` folds from empty.

## 14. The close check refuses

```sh
"$CG" invariant close-check --store "$WORK/store" --case-space-id "$CS" \
  --base-revision-id "$(cur)" --actor-id actor:release-manager \
  --capability-id capability:release-durable-mutation --operation-scope-id "$CS" \
  --audience audit --source-boundary-id "$SB" \
  --validation-evidence-id evidence:changelog-0-9-0-entry --format json
```

```
closeable: False
  PASS  close:native-base-revision-matches
  PASS  close:native-source-boundary-declared
  PASS  close:native-no-hard-obstructions
  FAIL  close:native-completions-reviewed
  FAIL  close:native-morphisms-reviewed
  FAIL  close:native-evidence-accepted-or-waived
  PASS  close:native-projection-loss-declared
  PASS  close:native-policy-capability-gate
  PASS  close:native-validation-evidence-named
```

The conclusion is the one the case space was built to reach: **0.9.0 is not
releasable**, because a worker demonstrated that the crate declares 0.8.0 and
that evidence cannot satisfy the tag step's hard requirement. Three unreviewed
generated artifacts — the completion candidates, the worker-evidence morphism,
and the pre-tag requirement — still await a human decision.

`close:native-policy-capability-gate` and `close:native-validation-evidence-named`
need `--validation-evidence-id`; it populates the request's `source_ids`, which
the gate invariant also requires. `close:native-no-hard-obstructions` passes
here because the remaining obstruction is `medium` — that invariant only counts
`high` and `critical`.

## What the run teaches

Two things worth knowing before modelling a real case space:

1. **A plan-level success requirement gates the transition, not readiness.**
   If a gate must block downstream work, wire it as a hard `requires_evidence`
   relation as well (§12). A plan alone leaves the frontier open.
2. **Opting out of worker execution still costs an attempt.** The
   `--enable-worker` refusal happens after the run directory is reserved and a
   trace is anchored, so it advances the revision and forces `--retry-step`
   (§5). That is the documented "failures after reservation retain a trace"
   behaviour, but it surprises on first contact.

And the controls that held, each demonstrated above rather than asserted:
declared trust values were discarded and re-measured (§3, §8); a capability
could not be self-granted (§7); an edited pinned script never ran (§11);
promotion required a review morphism and left the cell untouched (§8); an
illegal lifecycle transition was refused (§9); a stale base revision was refused
rather than merged (§7); and a failing gate was a finding, not a crash (§10).

## Files

| File | Role |
|---|---|
| [`release-decision/genesis.case.space.json`](release-decision/genesis.case.space.json) | The lift input: goal, two work cells, two requirement placeholders, four capability cells |
| [`release-decision/gate-schema-ids.sh`](release-decision/gate-schema-ids.sh) | Worker: every shipped schema carries an `$id` |
| [`release-decision/tag-dry-run.sh`](release-decision/tag-dry-run.sh) | Worker: `Cargo.toml` declares the version being tagged |

Bindings, plans, and morphism inputs are small JSON documents; their shapes are
in [`schemas/casegraphen/`](../../schemas/casegraphen/) with an `*.example.json`
next to each schema.

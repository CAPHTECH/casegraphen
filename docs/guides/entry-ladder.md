# The entry ladder

CaseGraphen has a real, deliberately lightweight entry — ADR 0003 §4 designed
it — but until now no shipped artifact showed it, so the only visible way in
was the [complete control model](release-decision-walkthrough.md): 14
sections, 17 commands, four capabilities, a worker, evidence promotion, tamper
detection. That walkthrough is still the reference for everything the tool can
do. It is not where you start.

Start here instead. Two rungs, cheapest first, both runnable verbatim against
a built binary and both guarded by [`tests/entry_ladder_conformance.rs`](../../tests/entry_ladder_conformance.rs)
so they cannot drift from what the tool actually does.

## Rung 1 — the analysis loop: zero authority

`lift workflow` materializes a `highergraphen.case.workflow.graph.v1` document
into a case space (ADR 0003 §4). A lifted workflow space contains **no
capability cells** — it satisfies no operation gate and cannot be mutated. It
exists only to be reasoned over: derived readiness, the frontier, and
obstructions, with nothing to author but the graph itself.

[`entry-ladder/mini-workflow.graph.json`](entry-ladder/mini-workflow.graph.json)
is the whole input — two work items and one `depends_on` relation, copied here
verbatim:

```json
{
  "schema": "highergraphen.case.workflow.graph.v1",
  "schema_version": 1,
  "workflow_graph_id": "workflow_graph:mini",
  "case_graph_id": "case_graph:mini",
  "space_id": "space:mini",
  "work_items": [
    {
      "id": "work_item:design",
      "space_id": "space:mini",
      "item_type": "task",
      "title": "Design the mini feature",
      "state": "doing",
      "case_ids": [],
      "hard_dependency_ids": [],
      "external_wait_ids": [],
      "evidence_requirement_ids": [],
      "proof_requirement_ids": [],
      "source_ids": [],
      "provenance": {
        "source": { "kind": "human", "title": "Mini example" },
        "confidence": 1.0,
        "review_status": "unreviewed"
      },
      "metadata": {}
    },
    {
      "id": "work_item:implement",
      "space_id": "space:mini",
      "item_type": "task",
      "title": "Implement the mini feature",
      "state": "todo",
      "case_ids": [],
      "hard_dependency_ids": ["work_item:design"],
      "external_wait_ids": [],
      "evidence_requirement_ids": [],
      "proof_requirement_ids": [],
      "source_ids": [],
      "provenance": {
        "source": { "kind": "human", "title": "Mini example" },
        "confidence": 1.0,
        "review_status": "unreviewed"
      },
      "metadata": {}
    }
  ],
  "workflow_relations": [
    {
      "id": "relation:implement-depends-on-design",
      "relation_type": "depends_on",
      "from_id": "work_item:implement",
      "to_id": "work_item:design",
      "evidence_ids": [],
      "source_ids": [],
      "provenance": {
        "source": { "kind": "human", "title": "Mini example" },
        "confidence": 1.0,
        "review_status": "unreviewed"
      }
    }
  ],
  "readiness_rules": [],
  "evidence_records": [],
  "transition_records": [],
  "projection_profiles": [],
  "correspondence_records": [],
  "metadata": {}
}
```

Every field here is required — the required-but-empty arrays
(`case_ids`, `evidence_requirement_ids`, `readiness_rules`, …) are the schema's
own cost, not something an example can hide; see ADR 0003's non-goals. What the
example removes is guessing them: this graph is copy-pasteable and lifts on
the first try.

Run it, verbatim, from a clean scratch store (from the repository root, after
`cargo build`):

```sh
CG=target/debug/casegraphen
STORE=$(mktemp -d)/store

"$CG" lift workflow --store "$STORE" \
  --input docs/guides/entry-ladder/mini-workflow.graph.json \
  --revision-id revision:mini-genesis --format json
```

**Read the created space id from the report, not the log.** `lift workflow`
derives the id from `workflow_graph_id` — here, `case_space:workflow_graph:mini`
— and the JSON report states it plainly at `result.case_space.case_space_id`.
You do not need to open `morphism_log.jsonl` to find it:

```sh
CS=$("$CG" lift workflow --store "$STORE" \
  --input docs/guides/entry-ladder/mini-workflow.graph.json \
  --revision-id revision:mini-genesis --format json \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["case_space"]["case_space_id"])')
echo "$CS"
```

```
case_space:workflow_graph:mini
```

> **Do not pass `--case-space-id` or `--space-id` to `lift workflow`.** No
> `lift` adapter accepts either — both ids are always derived from the input
> (`workflow_graph_id` here), never from what you name on the command line, so
> the flags are refused rather than honoured. The point above is how to read
> the derived id, not a way to choose your own.

Now reason over the lifted space — no gate, no capability, no worker, no
evidence, only what the graph declared:

```sh
"$CG" space reason --store "$STORE" --case-space-id "$CS" --format text
```

```
Progress: blocked
Assurance: review_required

Frontier:
  - work_item:design

Waiting:
  (none)

Obstructions:
  - obstruction:unresolved-dependency:work-item-implement:work-item-design: work_item:implement depends on unresolved cell work_item:design.
    witnesses: work_item:design

Unaccepted evidence findings:
  (none)

Review gaps:
  - unreviewed_completion: 1 gap(s) — Completion candidates remain reviewable findings until explicitly accepted or rejected.
    targets: completion_candidate:missing-dependency-resolution:obstruction-unresolved-dependency-work-item-implement-work-item-design

Completion candidates:
  - completion_candidate:missing-dependency-resolution:obstruction-unresolved-dependency-work-item-implement-work-item-design: A hard dependency must be resolved before downstream readiness.
    targets: work_item:design, work_item:implement
```

Two commands, both run above verbatim against the shipped example. The
dependency and readiness structure derives normally; nothing here is trusted
as a fact, and nothing here can be mutated — `space new` and a lifted workflow
space are both permanently read-only, by construction (see
[`authoring.md`](../../skills/casegraphen-operate/references/authoring.md)).
To act on any of this, go to rung 2.

## Rung 2 — the minimal governed loop: one gate, nothing else

Making workflow-originated work *executable* means authoring a native genesis
with explicit capabilities — ADR 0003 calls this "an intentional
re-declaration of authority, not a flag." The smallest version of that
declaration is one work cell and one capability cell.

[`entry-ladder/mini-genesis.case.space.json`](entry-ladder/mini-genesis.case.space.json)
is 108 lines: one `work` cell, one `custom:capability` cell naming the one
actor and the operations it may perform, one genesis morphism-log entry naming
who and from what, and the source boundary the gate checks against. It carries
**no derived copy** — `case_cells` and `case_relations` are the only content
you author; `lift native` derives the morphism payload, `added_ids`, and every
checksum from them, every time (see "The genesis snapshot is made
self-reconstructing for you" in `authoring.md`).

```sh
STORE2=$(mktemp -d)/store
CS2=case_space:mini-governed

"$CG" lift native --store "$STORE2" \
  --input docs/guides/entry-ladder/mini-genesis.case.space.json \
  --revision-id revision:mini-genesis --format json
```

```
result.case_space.case_space_id: case_space:mini-governed
result.case_space.case_cells:    [work:mini-task, capability:mini-operator]
```

That is the whole authoring step: no hash, no audit record, no handoff
document. Everything above satisfies the schema on the first attempt because
the example already carries the two fields a hand-authored minimal genesis
tends to miss first — `metadata.lift_semantics` and `metadata.source_boundary`
on the genesis morphism (`native_eval` refuses a genesis missing either).

From here, a small change is **one invocation**. `cell transition` moves
`work:mini-task` to `resolved`, presenting the one capability granted at
genesis and the source boundary it declared:

```sh
"$CG" cell transition --store "$STORE2" --case-space-id "$CS2" \
  --base-revision-id revision:mini-genesis \
  --cell-id work:mini-task --to resolved \
  --actor-id actor:mini-operator --capability-id capability:mini-operator \
  --operation-scope-id "$CS2" --audience audit \
  --source-boundary-id source_boundary:mini-governed --format json
```

```
result.entry.morphism.metadata.operation_gate:
{
  "actor_id": "actor:mini-operator",
  "audience": "audit",
  "capability_ids": ["capability:mini-operator"],
  "operation": "cell-transition",
  "operation_scope_id": "case_space:mini-governed",
  "source_boundary_id": "source_boundary:mini-governed"
}
```

The operator authored none of that gate metadata, the hash chain, or the
revision id that advanced — the tool minted all three. Confirm the space
reads as done:

```sh
"$CG" space reason --store "$STORE2" --case-space-id "$CS2" --format text
```

```
Progress: complete
Assurance: unreviewed
...
```

One lift, one transition: two invocations, zero retries, one gate. A
structural change (adding a cell or relation instead of transitioning one) is
the same shape — one morphism JSON naming the change, then
`morphism propose` + `morphism apply` — and is covered in full in the
walkthrough's [§7, "Change the structure while the work is in flight"](release-decision-walkthrough.md#7-change-the-structure-while-the-work-is-in-flight).

## Rung 3 — the complete model

The [release-decision walkthrough](release-decision-walkthrough.md) is
rung 3: multiple actors, worker dispatch, untrusted worker output, evidence
promotion, a close policy, tamper detection. Read it once you need any of
those — not as the way in.

## What's optional at each rung

| Surface | Rung 1: analysis | Rung 2: minimal governed | Rung 3: complete |
|---|---|---|---|
| Capability cells / gates | none — the space can't be mutated | one capability, one actor, one operation | multiple capabilities, actors separated by operation (ADR 0007) |
| Plans (`plan accept`, `run --step`/`--frontier`) | not applicable | not used | used |
| Workers / dispatch | not applicable | not used | used |
| Evidence attach / promote | not applicable | not used | used |
| Packets | not applicable | not used | used |
| Close policy | not applicable | not declared (`close_policy_id` omitted) | declared |

Every surface past rung 2 is pay-when-declared: a capability's
`metadata.operations` list only ever grants what it names, a case space
without a `close_policy_id` never engages close checking, and nothing below
rung 3 requires a worker binding to exist.

## Decided: no mechanical transcription assist, for now

Whether the analysis-to-executable step deserves tooling to transcribe a
lifted space's cells and relations into a genesis draft was an open question
this issue raised. It is decided, not deferred by omission — see
[ADR 0035](../adr/0035-no-analysis-to-genesis-transcription-assist.md).

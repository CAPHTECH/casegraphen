# Authoring a case space

## Lifting a workflow graph

`lift workflow` materializes a `highergraphen.case.workflow.graph.v1` document
into a case space (ADR 0003): work items become cells, workflow relations become
relations, evidence records become evidence cells. Nothing the graph declares
about trust survives — `case_ids` does not become `structure_ids`, the
evaluator-consulted metadata keys are stripped, and every lifted evidence cell
enters `unreviewed`. A lifted space carries no capability cells, so it is an
analysis space: to execute workflow-originated work, author a native genesis
with explicit capabilities.

## Use `lift native`, not `space new`

`space new` creates a case space containing one `case:native-root` cell and no
capability cells. Because capability cells can enter only in the genesis
materialization, **that space can never satisfy an operation gate** — every
durable mutation will fail with `capability … does not resolve to an existing
case cell`. It is permanently read-only.

Author a genesis snapshot instead and lift it:

```sh
casegraphen lift native --store "$STORE" --input genesis.case.space.json \
  --revision-id revision:<name>-genesis --format json
```

Start from the
[example genesis](https://github.com/CAPHTECH/casegraphen/blob/main/docs/guides/release-decision/genesis.case.space.json)
and edit it. The contract is
[`native.case.space.schema.json`](https://github.com/CAPHTECH/casegraphen/blob/main/schemas/casegraphen/native.case.space.schema.json)
(`additionalProperties: false` throughout).

## The genesis snapshot is made self-reconstructing for you

`space rebuild` folds the log from empty, so the single genesis log entry has to
carry the complete initial content. **You do not write that copy.** `lift native`
derives it from the top-level state you authored, every time, and reseals the
checksums around it. Write the state once:

- `case_cells` and `case_relations` — the complete initial content.
- One genesis log entry naming *who and from what*: `entry_id`, `morphism_id`,
  `target_revision_id`, `actor_id`, `recorded_at`, `provenance`, `source_ids`,
  and `morphism.metadata.source_boundary` whose `id` matches
  `metadata.source_boundary.id` at the top level. Gates check that id.

These are derived on lift. Whatever you supply is overwritten, not compared, so
a stale value is not a refusal — it is simply discarded:

| Field | Derived from | If you leave it out |
|---|---|---|
| `morphism.metadata.payload.added_cells` / `.added_relations` | the top-level `case_cells` / `case_relations` | fine — `metadata` is free-form |
| `morphism.metadata.genesis_case_space` | `space_id`, `projections`, `close_policy_id`, `metadata`, `revision_metadata` | fine — same |
| `morphism.added_ids` | every id in that payload | write `[]`; the field itself is required |
| `revision.checksum`, the entry's `replay_checksum` | the resealed space | write `""`; the fields are required |

Hand-mirroring the payload is the single most expensive way to get this wrong:
the copies are not compared, so a hand-written one is discarded, and a generator
that produces it is work the tool already did. The redundancy becomes load-bearing
only *after* the import — from the first appended morphism on, the hash chain is
what makes the log reconstructive.

## Capability cells are the authorization root

One `custom:capability` cell per distinct authority, `lifecycle: accepted`,
`provenance.review_status: accepted`, and `metadata.actor_ids` listing exactly
the actors that hold it. Separate the roles: the actor that accepts plans should
not be the actor that dispatches workers.

**`metadata.operations` is required and is what makes the separation real.**
List the operation strings the capability authorizes; the gate refuses when the
operation it is performing is not in that list, so a runner holding only a
dispatch capability cannot pass `review accept` with it. An absent or empty list
authorizes nothing — there is no permissive default, because a default that
means "every operation" is the separation you just modelled, silently undone.

```json
"metadata": {
  "actor_ids": ["actor:release-manager"],
  "operations": ["morphism-apply", "morphism-reject", "evidence-attach",
                 "cell-transition", "review"]
}
```

The strings are the ones the commands mint, listed under rule 2 in `SKILL.md`:
`plan-review`, `dispatch`, `morphism-apply`, `morphism-reject`,
`evidence-attach`, `cell-transition`, `review`, and `close-check`.

There is no CLI path to add, amend, or revoke one afterwards. Decide the grants
before lifting.

## Modelling readiness so it comes out right

Readiness, the frontier, and blockers are derived on every command. They are
driven by relations, so what you want enforced must exist as a relation.

| Intent | Model it as |
|---|---|
| B cannot start until A is done | `B --depends_on(hard)--> A`. Blocks while A is incomplete. |
| Advisory ordering only | the same relation with `relation_strength: soft`. Does not block. |
| B needs proof of something | `B --requires_evidence(hard)--> evidence:<placeholder>` plus a placeholder evidence cell. |
| B waits on an external event | `B --waits_for(hard)--> <cell>`. Satisfied by completion or by trusted evidence for that cell. |
| An event finishes a work item | `event --completes--> work`. Removes it from the frontier. |

**A cell counts as complete if its lifecycle is `resolved`, `accepted`,
`retired`, or `superseded` — or if its `provenance.review_status` is
`accepted`.** That second clause makes it easy to author a vacuous dependency:
mark a work cell `review_status: accepted` at genesis and every hard dependency
on it is satisfied before the work starts. Author work cells as `reviewed` (or
`unreviewed`) and reserve `accepted` provenance for facts, not for tasks. The
full rule set is under "Default readiness rules" in the
[native case-management spec](https://github.com/CAPHTECH/casegraphen/blob/main/docs/specs/casegraphen-native-case-management.md).

A **requirement placeholder** is an evidence cell with `lifecycle: proposed`,
`review_status: unreviewed`, and **no** `metadata.evidence_boundary`. That
combination cannot satisfy a hard requirement, so the requirement reads as
unsatisfied until real evidence is attached and promoted. Give it a title that
states the condition ("Required: the release gate reports clean"), because that
title is what a reviewer sees in the obstruction.

Cells in `resolved`, `accepted`, `retired`, `rejected`, or `superseded` are not
readiness subjects and never appear on the frontier. Evidence, review,
projection, revision, morphism, and external-ref cells are never readiness
subjects at all.

## The trap: a plan requirement is not a readiness requirement

A plan step's `success_evidence_requirement_ids` decides whether *that step's*
transition may be applied. It has no effect on readiness. A gate that fails will
still leave downstream work on the frontier unless the same requirement is also
a hard `requires_evidence` relation in the graph. If the gate must block, model
both.

## Vocabulary extension

`cell_type` and `relation_type` both accept `custom:<name>`, so a new kind of
node or edge needs no schema change. Everything else about a cell is fixed once
created: `cell_type` is immutable, and for evidence cells so are `provenance`
and `metadata.{evidence_boundary,content_hash,trace_id,worker_report_id}`.

## Check the model before building on it

```sh
# optional, with a local copy of the schema file
python3 -m jsonschema -i genesis.case.space.json native.case.space.schema.json
casegraphen space frontier   --store "$STORE" --case-space-id "$CS" --format json
casegraphen obstruction list --store "$STORE" --case-space-id "$CS" --format json
```

`lift native` rejects a malformed snapshot anyway, so schema validation is only a
faster feedback loop.

If the frontier or the blockers are not what you intended, fix the model now —
after the first mutation, changing it costs a gated morphism.

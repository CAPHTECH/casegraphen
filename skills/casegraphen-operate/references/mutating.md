# Changing the graph

Every change is an append to the morphism log. There is no in-place edit.
Re-read the current revision before each command; `$STORE`, `$CS`, `$GATE`, and
`cur()` below are from SKILL.md.

## Which command for which change

| Change | Command |
|---|---|
| Add, update, or retire cells and relations | `morphism propose` → `check` → `apply` |
| Record new evidence | `evidence attach` |
| Promote evidence, or accept/reject/reopen/waive a target | `review accept|reject|reopen|waive` |
| Move one cell's lifecycle | `cell transition` |
| Discard a proposal | `morphism reject` |

Prefer the specific command when one exists — it builds the canonical morphism
for you and stores the gate in it. A generic morphism cannot use the reserved
metadata keys `native_review_schema_version`, `target_kind`,
`outcome_review_status`, or `operation_gate`; those belong to the review paths.

## Generic morphism: propose → check → apply

The proposal is a `CaseMorphism` JSON document. The fields that must agree:

- `source_revision_id` — the current revision at apply time.
- `added_ids` / `updated_ids` / `retired_ids` — exactly the ids in
  `metadata.payload.{added,updated}_{cells,relations}`; `retired_ids` names
  existing ids. No id may appear in two lists.
- `morphism_type` — `create`, `update`, `relate`, `unrelate`, `retire`,
  `migration`, or `custom:<name>`. It is metadata for readers plus the key the
  plan's `allowed_transition_classes` match on; the reducer keys off the payload.
- `review_status: unreviewed` on anything generated rather than human-authored.

```sh
casegraphen morphism propose --store "$STORE" --case-space-id "$CS" --input m.json --format json
casegraphen morphism check   --store "$STORE" --case-space-id "$CS" --morphism-id <id> --format json
casegraphen morphism apply   --store "$STORE" --case-space-id "$CS" --morphism-id <id> \
  --base-revision-id "$(cur)" --reviewer-id <id> --reason "<why>" $GATE --format json
```

`check` reports `applicable` plus diagnostics and mutates nothing. Run it: it
catches payload/id mismatches before the gated call.

What the reducer refuses, at propose time:

- creating, updating, retiring, or transitioning a `custom:capability` cell
- changing a cell's `cell_type`
- changing an evidence cell's `provenance` or its `evidence_boundary`,
  `content_hash`, `trace_id`, `worker_report_id` metadata
- an illegal lifecycle transition (table below)
- leaving a relation whose `from_id` or `to_id` cell does not exist

Retiring a **cell** sets `lifecycle: retired` and keeps it as a tombstone;
retiring a **relation** removes it. Cells are never deleted.

## Legal lifecycle transitions

| From | To |
|---|---|
| `proposed` | `active`, `rejected`, `retired` |
| `active` | `waiting`, `resolved`, `retired`, `superseded` |
| `waiting` | `active`, `retired` |
| `resolved` | `accepted`, `active`, `retired` |
| `accepted` | `superseded`, `retired` |
| `rejected`, `superseded` | `retired` |

Same-to-same is allowed. `retired` is terminal. `proposed → resolved` is not a
transition; go through `active`.

## Attaching evidence is not promoting it

`evidence attach` takes an evidence cell document. The tool overwrites
`metadata.evidence_boundary` with `inferred` — the spelling that means "needs an
accepted review" — and sets `metadata.content_hash` from the input bytes, and it
refuses input claiming `review_status: accepted`. `--satisfies <target-id>` adds a
`satisfies_evidence_requirement` relation at `diagnostic` strength — which does
not satisfy a hard requirement.

```sh
casegraphen evidence attach --store "$STORE" --case-space-id "$CS" \
  --base-revision-id "$(cur)" --input evidence.json --satisfies <requirement-id> $GATE --format json
```

Expect the `missing_evidence` obstruction to survive this. To clear it, promote
the requirement:

```sh
casegraphen review accept --store "$STORE" --case-space-id "$CS" \
  --target-id <requirement-id> --reviewer-id <id> --reason "<what you verified>" \
  --base-revision-id "$(cur)" --evidence-id <attached evidence id> $GATE --format json
```

The review does **not** edit the target cell — its `provenance.review_status`
still reads `unreviewed` afterwards. Promotion lives in the review morphism and
the evaluator consults the log. Do not "fix" that by updating the cell; an
evidence cell's provenance is immutable and the update is refused.

## Direct lifecycle change

```sh
casegraphen cell transition --store "$STORE" --case-space-id "$CS" \
  --base-revision-id "$(cur)" --cell-id <id> --to <lifecycle> --reason "<why>" $GATE --format json
```

Capability-gated, so no human interaction is required if the acting actor holds
the capability. Use it for judgments a human made; use `run --step` for work a
worker did.

# Changing the graph

Every change is an append to the morphism log. There is no in-place edit.
Start a session with `REV="$(cur)"`. After each successful durable write, take
the next revision from that command's report; do not re-read between successful
writes. After a refusal or failure, recover with `REV="$(cur)"` before retrying.
`$STORE`, `$CS`, `$GATE`, and `cur()` below are from SKILL.md.

```sh
next_revision() {
  python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["result"]["record"]["current_revision_id"])' "$1"
}
```

## Which command for which change

| Change | Command |
|---|---|
| Add or update cells and relations; retire cells | `morphism propose` → `check` → `apply` |
| Record new evidence, with or without the file it is about | `evidence attach` |
| Drive attach → review pause → transition from one file | `packet apply`, then `packet resume` |
| Promote evidence, or accept/reject/reopen/waive a target | `review accept|reject|reopen|waive` |
| Move one cell's lifecycle | `cell transition` |
| Discard a proposal | `morphism reject` |

Prefer the specific command when one exists — it builds the canonical morphism
for you and stores the gate in it. A generic morphism cannot use the reserved
metadata keys `native_review_schema_version`, `target_kind`,
`outcome_review_status`, or `operation_gate`; those belong to the review paths.

## Generic morphism: propose → check → apply

The proposal is a bare `CaseMorphism` JSON document — no `case_space_id`
wrapper, and any add/update payload lives at `metadata.payload`, not the top
level — validated against `highergraphen.case.morphism_propose_input.v1`.
Fetch the shipped add-one-cell template rather than assembling one from this
section:

```sh
casegraphen schema get --file native.morphism-propose-input.example.json --format json \
  | jq -r .result.content > m.json
```

(`--id` selects a `*.schema.json` entry only, never an example — `--file` is
how the template itself gets fetched.)

`added_ids`, `updated_ids`, `retired_ids`, `preserved_ids`,
`violated_invariant_ids`, `evidence_ids`, and `source_ids` may all be left out
of `m.json` — the tool defaults each of them to `[]`. Fields that still must
agree once you edit the template:

- `source_revision_id` — the current revision at apply time.
- `added_ids` — if you leave it out (or leave it empty), the tool derives it
  from `metadata.payload.added_cells`/`added_relations`. If you declare a
  non-empty value yourself, it must match the payload exactly.
- `updated_ids` — **not derived**: if `metadata.payload` has
  `updated_cells`/`updated_relations`, `updated_ids` must name exactly those
  ids yourself; omitting it only works when there is nothing to update.
- `retired_ids` — names existing ids to retire; also not payload-derived.
- No id may appear in two lists.
- `morphism_type` — `create`, `update`, `relate`, `unrelate`, `retire`,
  `migration`, or `custom:<name>`. It is metadata for readers plus the key the
  plan's `allowed_transition_classes` match on; the reducer keys off the payload.
- `review_status: unreviewed` on anything generated rather than human-authored.

```sh
casegraphen morphism propose --store "$STORE" --case-space-id "$CS" --input m.json --format json
casegraphen morphism check   --store "$STORE" --case-space-id "$CS" --morphism-id <id> --format json
APPLY_REPORT=apply-report.json
casegraphen morphism apply   --store "$STORE" --case-space-id "$CS" --morphism-id <id> \
  --base-revision-id "$REV" --reviewer-id <id> --reason "<why>" $GATE --format json \
  --output "$APPLY_REPORT"
REV="$(next_revision "$APPLY_REPORT")"
```

`check` reports `applicable` plus diagnostics and mutates nothing. Run it —
and know its boundary: `check` can answer everything derivable from the
candidate graph the proposal would produce. It cannot answer who is asking,
or when the write lands. Concretely, `apply` additionally requires three
things `check` never sees:

- **The operation gate.** `check` takes no `--actor-id`/`--capability-id`/
  `--operation-scope-id`/`--audience`/`--source-boundary-id` — it cannot
  evaluate authorization because it does not know who is asking.
- **That `--base-revision-id` still names the current revision.** `check`
  takes no revision flag either, so a case space that moved since `check` ran
  invalidates its answer; there is no staleness `check` could have detected.
- **The append-time evaluator backstop**, which re-checks the log's whole
  history for self-consistency, not just this one candidate morphism.

`applicable: true` means the candidate graph is self-consistent by every rule
`check` runs; it is not a guarantee that `apply` will succeed.

What the reducer refuses, at propose time:

- creating, updating, retiring, or transitioning a `custom:capability` cell
- changing a cell's `cell_type`
- changing an evidence cell's `provenance` or its `evidence_boundary`,
  `content_hash`, `trace_id`, `worker_report_id` metadata
- an illegal lifecycle transition (table below)
- leaving a relation whose `from_id`, `to_id`, or own `evidence_ids` names an
  id that does not exist, or a projection whose represented/omitted cell or
  relation ids do not
- retiring a relation (below)

Retiring a **cell** sets `lifecycle: retired` and keeps it as a tombstone.
Retiring a **relation** is refused, at `propose`, not just at `apply`: unlike
a cell it gets no tombstone, so removing it would leave the morphism that
originally added it pointing at a missing id, which the store refuses. Do not
retire a relation to discharge the obstruction it participates in — keep the
relation and record the decision instead: `review waive --target-id
<obstruction-id>` on the obstruction (its id, from `space reason` or
`obstruction list` — not the relation's, and not the blocked cell's) accepts
the risk — the close-check invariants stop blocking on it — without erasing
the conflict: the obstruction stays listed in `space reason` afterward, by
design. Cells are never deleted.

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

`evidence attach` takes one or more evidence cell documents. Repeat `--input
<path> [--satisfies <target-id>]...`; each `--satisfies` belongs to the most
recent input. The tool overwrites each input's `metadata.evidence_boundary` with
`inferred` — the spelling that means "needs an accepted review" — computes each
`metadata.content_hash` from that input's bytes, and refuses input claiming
`review_status: accepted`. Each target adds a
`satisfies_evidence_requirement` relation at `diagnostic` strength — which does
not satisfy a hard requirement. The whole invocation appends one morphism and
one revision. If any input is refused, no input is appended.

```sh
ATTACH_REPORT=attach-report.json
casegraphen evidence attach --store "$STORE" --case-space-id "$CS" \
  --base-revision-id "$REV" \
  --input test-results.json --satisfies <test-requirement-id> \
  --input changelog.json --satisfies <changelog-requirement-id> $GATE \
  --format json --output "$ATTACH_REPORT"
REV="$(next_revision "$ATTACH_REPORT")"
```

## Attach the file, not just the claim about it

Add `--artifact <path>` to an input group to record the observed object itself —
a log, a test bundle, a document. The tool hashes the file and mints a
`custom:artifact` cell whose id *is* that hash (`artifact:sha256-<hex>`), plus a
`derives_from` relation from your claim to it, inside the same morphism. Two
citations of identical bytes land on one artifact cell.

```sh
casegraphen evidence attach --store "$STORE" --case-space-id "$CS" \
  --base-revision-id "$REV" \
  --input test-claim.json --satisfies <requirement-id> --artifact test-run.log \
  $GATE --format json --output "$ATTACH_REPORT"
```

Artifacts are observations, not claims: they are outside acceptability, they
never appear on the frontier, `review accept` on one is refused, and they cannot
be updated, transitioned, or retired. Review the claim that cites the artifact.
Artifacts enter only this way — a genesis snapshot or a morphism proposal
carrying one is refused, and a claim id inside the `artifact:sha256-` namespace
is refused.

Expect the `missing_evidence` obstruction to survive this. To clear it, promote
the requirement:

```sh
REVIEW_REPORT=review-report.json
casegraphen review accept --store "$STORE" --case-space-id "$CS" \
  --target-id <requirement-id> --reviewer-id <id> --reason "<what you verified>" \
  --base-revision-id "$REV" --evidence-id <attached evidence id> $GATE --format json \
  --output "$REVIEW_REPORT"
REV="$(next_revision "$REVIEW_REPORT")"
```

The review does **not** edit the target cell — its `provenance.review_status`
still reads `unreviewed` afterwards. Promotion lives in the review morphism and
the evaluator consults the log. Do not "fix" that by updating the cell; an
evidence cell's provenance is immutable and the update is refused.

An acceptance is wider than the target id you passed: every requirement the
attach recorded as covered goes live at once. `review accept` reports that set
as `result.activated_coverage` — read it, and report it, because it is what the
acceptance actually decided.

## A CI run is an artifact; "the gate is green" is the claim

Do not hand-write "CI passed" into an evidence cell. That records a claim about
a run the tool never saw — a caller-declared trust value with extra steps. Hand
the tool the run record instead and let it hash that:

```sh
gh run view <run-id> --repo <owner>/<repo> \
  --json databaseId,workflowName,headSha,conclusion,status,createdAt,updatedAt,url \
  > ci-run.json

casegraphen evidence attach --store "$STORE" --case-space-id "$CS" \
  --base-revision-id "$REV" \
  --input gate-claim.json --satisfies <requirement-id> --artifact ci-run.json \
  $GATE --format json --output "$ATTACH_REPORT"
```

The split is the one every artifact makes. `ci-run.json` is the observation:
content-addressed, immutable, outside acceptability. `gate-claim.json` is the
assertion someone reviews — "the release gate is green for this change" — and
`review accept` lands on it, never on the run record.

Two things the reviewer, not the tool, has to check:

- **The head sha binds the run to the code under decision, and nothing else
  does.** A green run for a different commit is the failure mode this recipe
  exists to make visible, so put the sha in the claim body and check it against
  what is actually being decided.
- **`conclusion: success` is the run's own report of itself** — a fact about
  what GitHub recorded, not a proof that the tests were adequate.

The tool does not fetch. It hashes what it is handed, which needs no network
client, no credentials, and no new trust surface — and it is more truthful than
a fetch would be about who read what.

## Direct lifecycle change

```sh
TRANSITION_REPORT=transition-report.json
casegraphen cell transition --store "$STORE" --case-space-id "$CS" \
  --base-revision-id "$REV" --cell-id <id> --to <lifecycle> --reason "<why>" $GATE \
  --format json --output "$TRANSITION_REPORT"
REV="$(next_revision "$TRANSITION_REPORT")"
```

Capability-gated, so no human interaction is required if the acting actor holds
the capability. Use it for judgments a human made; use `run --step` for work a
worker did.

## One packet instead of attach-then-transition by hand

A packet is a strict-JSON file naming a target transition, one claim, its
artifacts, and the completion reason. It drives the same attach and the same
transition — and it **always stops in between**, because one invocation carries
one gate, so one actor, and an actor that reviewed its own claim would be
self-accepting (ADR 0015).

```sh
casegraphen packet apply --store "$STORE" --case-space-id "$CS" \
  --base-revision-id "$REV" --packet packet.json $GATE \
  --format json --output "$APPLY_REPORT"
# result.status: paused_for_review
# result.halt.halt: needs_review — this pause is a producer of the shared
#   ADR 0016 halt object; completed_through and next_operations live inside
#   it, not duplicated at the top level
# result.halt.completed_through: the revision the attach produced — carry it
# result.halt.next_operations: structured fields for the two calls to make next
```

Then a **different actor**, holding a capability that lists the `review`
operation, runs `review accept` on `result.claim_cell_id`. Only after that:

```sh
casegraphen packet resume --store "$STORE" --case-space-id "$CS" \
  --base-revision-id "$REV" --packet packet.json \
  --completed-through <the apply revision> $GATE \
  --format json --output "$RESUME_REPORT"
```

`--completed-through` is an assertion, not a lookup: resume refuses if that
revision is not in this space's history, if the claim is not the evidence that
exact revision attached, or if no accepted review for it exists in the log. A
stored `review_status: accepted` on the cell does not count — only a review
morphism does.

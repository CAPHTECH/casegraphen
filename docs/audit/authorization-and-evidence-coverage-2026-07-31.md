# Authorization and evidence-coverage audit — 2026-07-31

Date: 2026-07-31
Scope: the operation gate and the hard-evidence-requirement decision, as reached
through CLI-only inputs. No direct writes into a store directory were used;
that is documented residual risk 2 and was deliberately left alone.
Method: adversarial review of the working tree, with every finding below
re-executed by hand against a real store before it was written down.

Baseline for every reproduction: `space validate` reported `valid: true` after
the attack, and `space rebuild` succeeded. None of these corrupt a store. They
grant authority the security policy says only a review grants.

One finding from the same rounds — a caller-declared `evidence_boundary` on an
added evidence cell — is fixed in `8984e78` and is not repeated here.

## 1. A capability authorizes its holder, not an operation

`check_operation_gate` (`src/native_review.rs:298`) checks that each
`--capability-id` resolves to a case cell, that the cell is `custom:capability`,
that its lifecycle is `active` or `accepted`, that its provenance is `accepted`,
and that `metadata.actor_ids` contains the acting actor. Nothing compares the
capability to `gate.operation`. A capability cell carries no operation field, so
there is nothing to compare against: **any capability an actor holds admits
every gated operation that actor attempts.**

Reproduced with the shipped walkthrough's own role split, in which
`actor:release-runner` holds only `capability:release-dispatch` and
`capability:release-worker-exec`:

```sh
casegraphen review accept --store "$STORE" --case-space-id "$CS" \
  --target-id evidence:changelog-updated --reviewer-id reviewer:runner \
  --reason 'self-approved by the runner' --base-revision-id revision:w1 \
  --actor-id actor:release-runner --capability-id capability:release-dispatch \
  --operation-scope-id "$CS" --audience audit \
  --source-boundary-id source_boundary:release-0-9-0-intent --format json
```
```
exit=0   new revision: revision:review:evidence~3achangelog-updated:2
```

The runner promoted evidence with its dispatch capability. This falsified
`skills/casegraphen-operate/references/authoring.md`, which promised that
separating the roles stopped exactly this; `f6a5c5e` corrected the promise and
left the behaviour, because scoping capabilities is a contract change. See
ADR 0007.

## 2. A hard evidence requirement has four independent ways in, and coverage is
   a mutable graph field

`trusted_evidence_exists` (`src/native_eval.rs:396`) is the whole decision. It
has six disjuncts; only the first asks about the requirement's own evidence
cell. The rest are satisfied by *any* already-trusted evidence cell being
re-pointed at the requirement, and the re-pointing is an ordinary gated write.

Setup for both reproductions: a `work` cell, an `evidence` cell that stays
`inferred` and unreviewed, and a hard `requires_evidence` relation between them.
Baseline in both stores: `missing_evidence` on `work:attack-target`.

**2a. One added relation, at diagnostic strength.** A `verifies` relation from
the example fixture's already-trusted `evidence:native-schema-json-valid` to the
unreviewed `evidence:needed`:

```
BASELINE                                                   ['missing_evidence']
AFTER a DIAGNOSTIC verifies relation from a trusted cell    []  <-- CLEARED
space validate valid: True
```

`trusted_evidence_relation_targets` (`src/native_eval.rs:419`) does not filter
`relation_strength`, unlike `direct_targets` (`src/native_eval/graph.rs:51`),
`completed_targets` (`:27`), and `contradiction_relations`
(`src/native_eval.rs:439`). `diagnostic` is the strength this tool itself mints
for untrusted links in `evidence attach` (`src/native_cli/ops/mutations.rs:110`)
and `run --step` (`src/native_cli/ops/run.rs:1880`), so the weakest edge the
tool produces satisfies the strongest requirement it models.

**2b. Widening a trusted evidence cell's `structure_ids`.** The cell is taken
verbatim from `space replay` and one id is appended; `provenance` and all four
frozen metadata keys are untouched, so
`require_immutable_cell_update_fields` (`src/native_model.rs:905`) passes:

```
trusted cell boundary: source_backed | review_status: accepted
structure_ids before: ['…native.case.space.schema.json', '…native.case.space.example.json']
AFTER widening structure_ids                                []  <-- CLEARED
space validate valid: True
```

`src/native_eval.rs:119` reads `structure_ids` of every trusted evidence cell as
"this evidence covers that id". That is a coverage claim, which is a trust
value, and it is writable after the review that promoted the cell — so a
promotion decision can be extended to cover work the reviewer never saw.

Two further routes were demonstrated by the review and are not reproduced here:
setting `evidence_ids` on the requirement relation through `updated_relations`,
and the same `structure_ids` widening reached through the relation disjuncts.

## 3. `updated_relations` has no reducer rule at all

`apply_morphism` (`src/native_model.rs:468`) and `apply_morphism_indexed`
(`:615`) look an updated relation up by id and overwrite it. Cells get
`require_immutable_cell_update_fields`, `require_lifecycle_transition`,
`require_not_capability_administration`, and now
`require_untrusted_added_evidence`. Relations get an existence check.
`relation_type`, `relation_strength`, `from_id`, `to_id`, `provenance`, and
`evidence_ids` are all rewritable, including on relations that entered at
genesis. The review demonstrated retyping a genesis `accepts` relation into a
hard `contradicts` between unrelated cells, and the same lever erases a
`contradiction` obstruction by retyping the relation that caused it — with no
waiver review.

`docs/security/worker-execution-policy.md` states the reducer's update rules for
cells and says nothing about relations, in either direction.

## 4. What this means for the policy

Two claims in `docs/security/worker-execution-policy.md` are literally true and
operationally empty while section 2 stands:

- "Inferred or worker-produced material never satisfies a hard evidence
  requirement until review promotes it" — the inferred cell never becomes
  trusted, and the requirement is cleared anyway.
- "Promoting worker evidence to satisfy a hard requirement | Always
  (`review accept`, with operation gate)" — reproduced with zero review
  morphisms.

The shape of the durable fix, for whoever takes this: trusted-evidence coverage
should be derived from the canonical review and attach morphisms in the log, the
way `latest_evidence_review_statuses` (`src/native_eval/sections.rs:533`) already
is, rather than read from graph fields a gated write can edit. Filtering
`relation_strength` and freezing `structure_ids` each remove one route; neither
removes the class, and shipping them alone would make the remaining routes
harder to find without making them harder to use.

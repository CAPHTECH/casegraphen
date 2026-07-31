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
   a mutable graph field — **fixed**

> **Resolved.** Coverage is now derived from the morphisms that minted it
> (`native_eval::sections::canonical_evidence_coverage`), so none of the routes
> below satisfies a requirement any more; the edges stay in the graph and stay
> visible, they just are not what the decision reads.
> `trusted_evidence_exists` went from six disjuncts to three, and every test in
> the suite still passes — the five graph-reading disjuncts were not load-bearing
> for any legitimate shape, which is what this section asked to be confirmed.
>
> Closing it exposed one more caller-declared trust value, found before the
> review round reported: coverage was keyed on `morphism_type ==
> EvidenceAttach`, and `morphism_type` is a field of a proposal file. Writing
> `evidence_attach` on the hand-authored morphism of 2a cleared the requirement
> again. `review` and `evidence_attach` are now reserved morphism types on
> generic propose/apply, next to the canonical review metadata keys that were
> already reserved for the same reason.
>
> The record below is kept as written, because the reasoning is what made the
> fix findable.

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
(`src/native_eval.rs:439`).

**Filtering it is not the fix, and would break the product.** The canonical
path runs through the same disjunct at the same strength. Reproduced:

```
BASELINE                                         ['missing_evidence']
after evidence attach (not yet reviewed)         ['missing_evidence']
after review accept (canonical promotion)        []  CLEARED

the relation the canonical path minted:
  satisfies_evidence_requirement | strength: diagnostic | to: evidence:needed
```

`evidence attach` mints `Diagnostic` deliberately
(`src/native_cli/ops/mutations.rs:110`), as does `run --step`
(`src/native_cli/ops/run.rs:1880`). Requiring `Hard` here would refuse every
piece of evidence this tool attaches. The attack and the promotion differ in
**who minted the coverage edge**, not in its strength or its type: one came
from an `evidence attach` morphism whose `--satisfies` target is recorded in the
log, the other from a generic `morphism apply`. Any fix that reads the edge out
of the current graph cannot tell them apart.

**2b. Widening a trusted evidence cell's `structure_ids`. Fixed.** The cell was
taken verbatim from `space replay` and one id appended; `provenance` and all
four frozen metadata keys were untouched, so
`require_immutable_cell_update_fields` (`src/native_model.rs:905`) passed:

```
trusted cell boundary: source_backed | review_status: accepted
structure_ids before: ['…native.case.space.schema.json', '…native.case.space.example.json']
AFTER widening structure_ids                                []  <-- CLEARED
space validate valid: True
```

`src/native_eval.rs:119` reads `structure_ids` of every trusted evidence cell as
"this evidence covers that id". That is a coverage claim, which is a trust
value, and it was writable after the review that promoted the cell — so a
promotion could be extended to cover work the reviewer never saw.

`structure_ids` is now immutable on evidence-cell updates, alongside the
provenance and metadata keys that rule already froze. This route is closed and
the canonical path is unaffected — `evidence attach` adds cells and never
updates one, and attach-then-`review accept` still clears the requirement. It is
a separable defect from the rest of section 2: the others are about *where
coverage is read from*, this one was about a completed review's subject changing
after the fact.

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

Two claims in `docs/security/worker-execution-policy.md` were literally true and
operationally empty while section 2 stood. Both hold operationally now:

- "Inferred or worker-produced material never satisfies a hard evidence
  requirement until review promotes it" — the inferred cell never became
  trusted, and the requirement was cleared anyway.
- "Promoting worker evidence to satisfy a hard requirement | Always
  (`review accept`, with operation gate)" — reproduced with zero review
  morphisms.

The fix as shipped, verified end to end: a generic morphism adding a
`verifies` edge from already-trusted evidence leaves `missing_evidence`
standing, and `evidence attach --satisfies` followed by `review accept` still
clears it — the first time the log records both a coverage claim and a review of
the evidence making it.

The reasoning that produced it, kept because the argument is the durable part:
**a coverage claim is
trusted when a canonical morphism minted it, not when it is present in the
graph.** `evidence attach --satisfies T` already records the claim in the log —
the attach morphism's payload carries the `satisfies_evidence_requirement`
relation from the cell to `T` — and `run --step` records the same shape for
worker output. Genesis carries its own coverage as the declared trust root.
Everything else is a generic write.

So the derivation is available from the log, the way
`latest_evidence_review_statuses` (`src/native_eval/sections.rs:533`) already
derives review status: build the set of coverage edges that entered through
genesis, `evidence attach`, or `run --step`, and have `trusted_evidence_exists`
consult that instead of reading `structure_ids`, relation targets, and
`evidence_ids` out of the current graph. The relation would still be in the
graph for display; it would stop being what the trust decision reads.

Both consequences that were flagged before implementing came out as predicted. A
space where a coverage edge was added by a generic morphism loses that
satisfaction and the obstruction reappears — fail-closed, the same direction as
`8984e78`. And the five graph-reading disjuncts turned out not to be load-bearing
for any legitimate shape: the whole suite passes with them gone, including the
worker dispatch, workflow-lift, github-lift, and close-check paths.

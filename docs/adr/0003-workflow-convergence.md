# ADR 0003: Converge The Workflow Family Into The Native Lift

## Status

Accepted on 2026-07-30. Executes option C for candidate 1 of
`docs/audit/local-optima-audit-2026-07-30.md`, which §11 deferred with the
note that the cost of converging only rises after publication. 0.8.0 is not
yet published; this is the last cheap moment.

## Context

The crate carries two model families. The native case space is the execution
substrate: one reducer, one store, one evaluator, gated mutations, a
reconstructive log. The workflow graph family is a second, non-executing
evaluator (~6,900 lines across `workflow_eval`, `workflow_report`,
`workflow_workspace`, its store and CLI bridge) with its own readiness rules,
its own report contracts, and its own store-backed review commands.

The audit's candidate 1 found the two families answering the same trust
question divergently. That defect was closed by unifying the rule into
`src/evidence_trust.rs`, but the structural duplication stayed: every future
hardening pass must remember to visit both families, and round 3 of the
adversarial review already demonstrated what happens to the family a pass
forgets. The audit recorded (§9) that nothing consumes the workflow surface —
HigherGraphen references it only in prose, and its one operational fixture is
a case space.

Meanwhile `lift workflow` — the sanctioned bridge between the families — is
shallow: it records source identity and boundary metadata but materializes no
cells, so a lifted workflow graph produces an empty case space that the native
evaluator can say nothing about. The two facts together mean the workflow
family survives *because* the bridge is too weak to replace it.

## Decision

1. **The workflow graph remains a wire contract, but only as lift input.**
   `workflow.graph.schema.json` and its example stay. Structural validation of
   the input (referential integrity across items, relations, evidence,
   histories) stays, single-sourced beside the model.

2. **`lift workflow` materializes.** Work items become case cells, workflow
   relations become case relations, evidence records become evidence cells
   with their boundaries normalized through the existing
   `EvidenceBoundary → EvidenceTrustBoundary` conversion. The lift writes a
   proper genesis: payload, matching `added_ids`, embedded case-space shell,
   declared source boundary — so the lifted space replays, rebuilds, and
   validates like any other.

3. **The second evaluator dies.** `workflow_eval`, `workflow_report`,
   `workflow_workspace`, the workflow store, the `workflow *` and
   `cg workflow *` CLI surfaces, the workflow topology paths, and the
   workflow report schemas are deleted. The reports those commands produced
   are now spelled: lift the graph, then use the native derived surface —
   `space reason`, `space frontier`, `obstruction list`, `space evidence`,
   `space topology`. Readiness over a workflow graph is thereby *derived by
   the one evaluator* instead of stored-and-recomputed by a second one.

4. **Lifted workflow spaces are read-only by construction.** The workflow
   vocabulary has no authorization concept, and inventing capability grants
   during lift would manufacture a trust root from caller input. So a lifted
   space contains no capability cells and cannot satisfy any operation gate:
   it is an analysis space. Making workflow-originated work *executable* means
   authoring a native genesis with explicit capabilities — an intentional
   re-declaration of authority, not a flag. The materialization asserts this
   rather than inheriting it from the type mapping being exhaustive.

5. **Nothing the caller declares about trust survives the lift.** A work item's
   `metadata` is copied except for the keys the evaluator and reducer read as
   trust inputs (`evidence_boundary`, `content_hash`, `trace_id`,
   `worker_report_id`); `case_ids` becomes `metadata.workflow_case_ids` and
   never `structure_ids`, because the evaluator reads an evidence cell's
   `structure_ids` as "this evidence covers that requirement"; and every lifted
   evidence cell enters `unreviewed`, so a review-promoted boundary needs a
   gated review morphism rather than the caller's own say-so. The genesis also
   records the SHA-256 of the input bytes, and the input is read once, so the
   recorded source identity and the materialized cells cannot describe
   different documents.

### Vocabulary mapping

Cell types map 1:1 except where the native vocabulary has no counterpart:

| WorkItemType | CaseCellType | | WorkflowRelationType | CaseRelationType |
|---|---|---|---|---|
| `task` | `work` | | `depends_on`, `waits_for`, `requires_evidence`, `requires_proof`, `verifies`, `blocks`, `contradicts`, `completes`, `derives_from`, `transitions_to`, `projects_to`, `corresponds_to`, `supersedes` | same name |
| `goal`, `decision`, `event`, `evidence`, `proof`, `case` | same name | | `relates_to` | `custom:relates_to` |
| `external_wait` | `external_ref` | | | |
| `review_action` | `review` | | | |
| `milestone` | `custom:milestone` | | | |

States map onto lifecycles with one deliberate discard:

| WorkItemState | CaseCellLifecycle |
|---|---|
| `proposed` | `proposed` |
| `todo`, `doing` | `active` |
| `waiting` | `waiting` |
| `blocked` | `active` — **stored blockedness is discarded.** Readiness is derived, never stored; if the graph's relations justify the block, the native evaluator re-derives it, and if they do not, the stored flag was an unsupported claim |
| `done` | `resolved` |
| `cancelled`, `failed` | `retired` |
| `accepted` | `accepted` |
| `rejected` | `rejected` |

The original state is preserved as `metadata.workflow_state` on every lifted
cell, so nothing is silently lost even where the mapping collapses
distinctions (`todo`/`doing`, `cancelled`/`failed`).

Workflow relations carry no strength; readiness-bearing types (`depends_on`,
`waits_for`, `requires_evidence`, `requires_proof`, `blocks`, `contradicts`)
lift as `hard`, annotative types as `diagnostic`, and the defaulting is
declared in the lift's information loss.

### Declared information loss

Recorded in the genesis `source_boundary.information_loss`, not silently
dropped: workflow readiness rules (the native evaluator's rules replace them),
transition records, completion review records, correspondence records, and
projection profiles (histories and profiles are not state; their counts and
ids are recorded), the `todo`/`doing`/`blocked` distinctions, and the
relation-strength defaulting.

## Consequences

- One evaluator, one store, one report surface. Roughly 6,400 lines and three
  report schemas leave the crate; a hardening pass has one family to visit.
- The CLI loses `workflow reason|validate|readiness|obstructions|completions|
  evidence|project|correspond|evolution|history topology` and all of
  `cg workflow *`. This is a breaking surface change made deliberately before
  first publication; backward compatibility was not a requirement.
- `workflow.report.schema.json`, `workflow.report.example.json`, and
  `workflow.operation.report.schema.json` are retired through the
  contract-change process (schema deletions are contract decisions).
- The typed-handoff obligation was discharged before first publication by
  deleting the inert execution-plan field. A future typed-handoff decision
  starts from a contract with no dead field and must introduce its contract
  and behavior together.
- The adversarial-execution-reviewer pass ran on the implementation and found
  five defects, each reproduced independently before the fix and re-attacked
  after. Four are recorded in decision 5 and its store counterparts: a cell
  colliding with a genesis structural id imported cleanly and then made every
  derived read fail permanently (the store now runs the evaluator's own
  contract at the import boundary); a failed import left a logless case
  directory that burned the case-space id and broke `space list` for the whole
  store (the write is now rolled back); caller-declared `case_ids` plus
  `metadata.evidence_boundary` satisfied both hard requirements of a lifted
  graph; and the legacy `accepted_evidence` label combined with a
  caller-declared accepted review promoted its own evidence. The fifth was the
  double read of the input file. `tests/command.rs` now carries a regression
  test per attack.
- Two single-source violations found by the same pass are closed: the
  `evidence_boundary` spelling now comes from
  `EvidenceTrustBoundary::metadata_value` at all three writers. Attached
  evidence therefore records `inferred` rather than the unrecognized
  `attached_unverified`, which the trust rule only happened to treat the same
  way.
- `docs/specs/casegraphen-workflow-reasoning-engine.md` and the report
  sections of `casegraphen-workflow-contracts.md` describe deleted machinery
  and are superseded by this ADR; the input-contract sections remain normative
  for the lift.

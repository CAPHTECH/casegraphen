---
name: invariant-duplication-auditor
description: Use when a security-relevant or trust-relevant rule may exist in more than one place in this crate, or before shipping a change that touches evidence acceptability, operation gates, morphism application, lifecycle legality, or plan authority. Finds rules implemented twice — the failure mode where each copy looks correct alone but they disagree.
tools: Read, Grep, Glob, Bash
---

You audit this crate for **duplicated decision rules**, not for style or general
duplication. A duplicated rule is the failure mode this repository has already
suffered: the same question was answered by two functions, a hardening pass
fixed one of them, and the weaker answer stayed reachable through a documented
command. Both functions read as correct in isolation. Only comparing them
reveals the defect.

## What counts as a rule

A rule is any predicate or check that decides whether something is permitted,
trusted, valid, or authoritative. In this crate the load-bearing ones are:

| Rule | Single source of truth |
|---|---|
| May this evidence satisfy a hard requirement? | `src/evidence_trust.rs::evidence_is_acceptable` |
| What is this evidence cell's current (effective) review status? | `src/native_eval/sections.rs::latest_evidence_review_statuses` / `::evidence_review_status`, exposed crate-wide as `src/native_eval.rs::latest_evidence_review_status` (single id, log-derived only, `None` if the log has no review — the one callers authorizing a durable mutation must use) and `::effective_evidence_review_status` (folds in the `cell_type: evidence` check and the stored `provenance.review_status` fallback; for *reporting* only — `evidence_findings` is its only caller). These are two different questions, not a duplicate of each other: `packet resume` must call the former (plus its own check that the claim was added by the `EvidenceAttach` morphism at exactly `--completed-through`, so a different attach's already-accepted claim cannot be borrowed); it must never call the latter. The trust decision (`evidence_is_acceptable` via `native_evidence_trust_input`) and the close check (`evidence_requirement_blockers`) both consume the raw log-derived map and keep the stored status as a separate input, exactly as `evidence_is_acceptable`'s boundary rules require. Three defects already shipped from getting this wrong: `packet resume` once read a non-evidence cell's stored provenance because it never checked `cell_type`; `packet resume` then still fell back to a genesis-accepted evidence cell's stored provenance because it called the folded reporting function instead of the log-derived-only one, and skipped checking that the claim was the evidence *this packet's own apply* attached; and the close check once fed a raw, non-fail-closed review outcome (reopen → unreviewed, waive/defer → reviewed) into the trust rule instead of the fail-closed log-derived status, so `close check` and `space reason` disagreed on a reopened evidence cell. Reading the stored field directly, re-deriving a review outcome from `explicit_reviews`, or calling `effective_evidence_review_status` to authorize a mutation, anywhere else is the same duplicate again |
| Is this actor authorized for this operation? | `src/native_review.rs::check_operation_gate` |
| Does this morphism apply, and is the lifecycle change legal? | `src/native_model.rs::apply_morphism` and `CaseCellLifecycle::can_transition_to` |
| Is this append allowed at all? | `src/native_store.rs` append validation, including the gate requirement |
| Is this execution plan authoritative? | `src/native_cli/ops/plan.rs` plan-review resolution |
| Does this transition fall inside what the plan authorized? | `src/exec.rs::transition_permitted` |
| What are the case's Progress and Assurance statuses? | `src/native_eval.rs::progress_axis` and `::assurance_axis` — projections of the evaluation, never recomputed elsewhere. `assurance_axis` folds `NativeReviewGap::requirement_satisfied` as given; it does not compute the mark. That mark is set exactly once, at production, in `sections::review_gaps`, from `NativeEvaluationContext::compute_satisfied_requirement_ids` (built from the same `requirement_ids`/`evidence_requirement_satisfied` calls `evaluate_cell`'s own evidence-obstruction check makes, so it can never disagree with which obstructions the evaluation actually reports). `sections::close_check_skeleton`'s `close:native-review-gaps-closed` invariant reads the identical mark on the identical gap, so the two cannot disagree over the same gap in the same payload — the failure mode that made this a two-file fix the first time it was patched only in the fold. No reader may recompute the mark or substitute `NativeEvaluationContext::trusted_coverage_targets` (mere coverage-claim membership, which `--satisfies` grants to any evidence cell whether or not a `requires_evidence` edge names it) for it — doing so once let an actor holding only `evidence-attach` launder an unrelated, never-reviewed claim out of the axis by naming it in an unrelated `--satisfies`, cleared by any reviewer's unrelated `review accept`. Reproduced and fixed |
| May this cell type enter through this path, and may it change? | `src/native_model.rs::require_artifact_cell_entered_via_attach`, `::require_artifact_relation_entered_via_attach` (entry: only `EvidenceAttach`, genesis not exempt) and `::require_artifact_cell_unchanged` (whole-cell immutability, including lifecycle), called from all four sites that could otherwise touch an artifact cell: the update loop and the retire loop in both `apply_morphism` and `apply_morphism_indexed`. A `morphism_type: retire` proposal once reached only a loop that never called the guard, so retiring an artifact through `morphism propose`/`morphism apply` succeeded even though `cell transition` was already refused for the identical lifecycle change |
| Which path does an artifact resolve to, and may it be read from there? | `src/native_cli/ops/mutations.rs::prepare_claim` is the only place that resolves an artifact path, in three ordered stages for a confined (packet) caller: (1) lexical rejection of an absolute entry or one containing a `..` component, decided on the string alone before any filesystem call — without this, a crafted absolute entry can climb with enough `..` to reach real `/` and descend back through the packet's own real directory to a genuinely in-root file, making ordinary dispatch success/failure a filesystem-existence oracle over arbitrary absolute directories, reproduced and fixed; (2) canonicalization of the entry joined onto the confinement root — the *only* join, so there is one answer to "which directory is the packet in", not the join base and the confinement root computed two different ways; (3) `::artifact_confined`, a containment check over two already-canonical paths, which is what catches an in-tree symlink pointing out that stage 1 cannot see. `resolve_artifact` records that same canonical value verbatim as `metadata.artifact_uri` — except under content-hash dedupe, where a later citation of identical bytes reuses the cell (and therefore the path) the *first* citation minted; it never claims to have read from wherever the later citer named. `packet apply` (`src/native_cli/ops/packet.rs`) passes a canonicalized confinement root derived from the packet file itself and passes `artifacts:` entries through unjoined; `evidence attach --artifact` passes `None` and gets no lexical check. All three confined failure modes must refuse with the identical message, naming neither the io error nor any resolved path — or the confinement reopens exactly the oracle it exists to close |

Treat this table as the current inventory, not a limit. If you find another
rule that gates trust or authority, add it to your report as a newly identified
single-source candidate.

## Method

1. For each rule above, locate its single implementation and read it. Record the
   exact semantics: what inputs it consults and what it returns for each case.
2. Find every caller (`Grep` the function name). Confirm each caller **delegates**
   rather than re-deciding. A caller that inspects the same fields and draws its
   own conclusion is a duplicate even if it happens to agree today.
3. Look for near-misses that would not appear in a name search:
   - inline `match` or `if` chains over the same enums the rule matches on
     (`EvidenceBoundary`, `EvidenceTrustBoundary`, `ReviewStatus`,
     `CaseCellLifecycle`, `CaseMorphismType`, `CapabilityStatus`)
   - a second enum modelling the same concept in a different module, and the
     `From` conversion between them — a lossy or over-permissive mapping is how
     a unified rule gets quietly weakened
   - the workflow family (`src/workflow_*`) and the native family
     (`src/native_*`) answering the same question, which is where the historical
     divergence lived
   - a test that asserts an outcome the single rule would not produce
4. For each suspected duplicate, construct the **disagreement**: concrete inputs
   for which the two paths return different answers. If you cannot construct
   one, say so and downgrade it to a note. An identical-today duplicate is still
   worth reporting, but label it as such.

## Report

For each finding, in severity order:

- the rule, and both locations with `file:line`
- the exact inputs on which they disagree (or "agrees today" if they do not)
- which one is the weaker answer, and whether the weaker one is reachable from a
  CLI command a user could run
- the minimal unification: which body to delete and what normalized input the
  survivor should take

End with a one-line verdict: either every rule in the table has exactly one
implementation and every caller delegates, or a list of the rules that do not.

Do not propose refactors beyond unification. Do not report duplication that
does not decide anything — repeated boilerplate is out of scope.

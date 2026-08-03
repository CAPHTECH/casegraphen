# Issue #76 implementation local-optima audit

## Scope and evidence

- `B`: release-evidence aggregation, runtime-family integration, and the
  independent non-Rust MCP topology-to-review client.
- `M`: missing, unavailable, timed-out, unreviewed, over-budget, or
  unretained evidence cannot qualify for promotion; runtime output never
  becomes accepted evidence directly.
- `N`: evaluation scripts, retained pilot evidence, tests, workflow, and
  operator documentation. Stable ledger decision rules are unchanged.
- `T`: provider execution, independent judgment, aggregation, later review,
  and topology redesign after a failed matrix.

Evidence planes:

1. Strict matrix tests execute the exact two-provider by ten-scenario baseline,
   missing/unavailable/timeout refusals, content-addressed retention, and a
   run-bound cost waiver with a reviewer-authorized limit.
2. Runtime pilot tests execute four distinct local runtime families. SQLite
   durable-queue and asyncio event-stream families additionally exercise
   generic JSONL artifact ingest and exact resource reconciliation.
3. A Python standard-library MCP client independently completes initialize,
   propose, lint, compile, attach, and reconcile, then stops at the review
   seam with `accepted: false`.
4. `docs/pilots/issue-76/retained-evidence.manifest.json` binds the generated
   JSONL streams and reports to SHA-256 digests and byte lengths.

## Observations

- [Evidence] The aggregator requires exactly Codex and Claude, all ten scenario
  IDs, pinned runner identity/version, the exact scenario-manifest hash, and
  the baseline evaluator kinds.
- [Evidence] Embedded summary results must equal retained per-scenario
  `result.json`; symlinked evidence is refused rather than followed.
- [Evidence] Manual judgments and cost waivers bind to both provider run content
  hashes. Each waiver has reviewer, reason, and positive `maximum_usd`, and
  cannot authorize a provider-declared budget above that limit.
- [Evidence] Release and pilot evidence are content-addressed; release failures
  emit only unreviewed audit/redesign proposals with `accepted: false`.
- [Evidence] Complete runtime reconciliation and the independent MCP client
  both stop at `needs_review`.
- [Inference] The shared generic JSONL/resource seam is exercised through four
  materially different transport/storage families, but this does not prove
  remote-provider behavior or sustained-load characteristics.

## Local rationality and compensation halo

Initially, trusting the aggregate summary was locally attractive because the
provider runner already generated it. That moved completeness verification to
human review: retained `result.json` files, provider identity, and manifest
binding could disagree while the aggregate appeared complete. Classification:
`externalization`, severity E2/A1/F2/K2/T1 = 8, confidence C3. The fix compares
retained results exactly and validates provider pins and manifest identity.

Walking retained evidence recursively was convenient, but following symlinks
would let a run inventory bytes outside its evidence directory. Classification:
`boundary inversion`, severity E2/A0/F2/K1/T1 = 6, confidence C3. The retention
step now refuses symlinks.

The baseline originally carried a manifest field without validating it, making
future scenario drift a downstream discovery. Classification: `temporal`,
severity E1/A1/F2/K1/T2 = 7, confidence C3. Baseline validation now checks the
manifest schema and its exact scenario/evaluator inventory.

Finally, treating “cost waived” as a provider boolean would optimize matrix
completion while exporting financial risk to the reviewer. Classification:
`externalization`, severity E2/A1/F2/K2/T2 = 9, confidence C3. The waiver is now
independent-reviewer authored, bound to exact run hashes, reasoned, and capped;
the provider's declared maximum must fit inside that cap.

## Counterfactuals

- A — Keep per-provider summaries and rely on a release reviewer to find
  omissions: small implementation, large compensation halo. Rejected.
- B — Add only matrix counting: catches missing rows but not substituted files,
  stale reviews, cost authority, or evidence provenance. Rejected.
- C — Validate exact retained evidence, bind independent decisions to run
  hashes and limits, retain content-addressed reports, and keep all outputs
  unaccepted until review. Adopted.

## False positives and residual risks

- [Evidence] Similar request/response logic in the runtime pilot and independent
  MCP client is intentional implementation independence, not accidental rule
  duplication; neither owns CaseGraphen decision semantics.
- [Evidence] The aggregate workflow not passing without an external manual
  review is the intended trust boundary, not a CI availability defect.
- [Hypothesis] Real Codex and Claude release executions may reveal provider-
  specific output or cost-telemetry differences. No real provider secret was
  consumed for this implementation; the workflow and guide prepare the exact
  commands.
- [Hypothesis] Four local families do not establish production durability,
  remote transport security, or performance under load. Promotion remains
  false in the retained report.

Within the issue boundary, no material local optimum remains after widening
the evaluation boundary from individual runner success to exact retained
matrix evidence, independent review authority, and post-run provenance. This
is not a claim of a global optimum or stable-contract readiness.

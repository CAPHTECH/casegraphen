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
- [Evidence] Pinned profiles classify only known non-API CLI sessions; the
  workflow assigns provider-specific hosts and injects no provider key. A
  summary remains a caller assertion until a broker HMAC binds its exact run,
  session class, challenge, and host boundary.
- [Evidence] Provider execution does not optimize completion by bypassing tool
  permissions: Codex is workspace-write/ephemeral with user config ignored,
  while Claude is limited to project Read/Write/Edit without ambient MCP.
- [Evidence] A provider-reported resolved model must equal the requested model;
  an accepted alias that resolves elsewhere fails instead of masquerading as
  exact model evidence.
- [Evidence] Evidence review, stale-revision handling, and failure/halt recovery
  now have deterministic output contracts. Their manual judgments were narrowed
  to qualitative rationale so reviewers do not re-evaluate the exact fields.
- [Evidence] Checkout, dependency installation, and evaluator build occur in an
  uncredentialed hosted prepare job. Authenticated provider runners consume
  only the prepared artifact, use an absolute evaluator path, and bind workflow
  inputs through quoted environment-derived argv.
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

After the authentication contract was clarified, checking only package and
version would have let an API-authenticated or unauthenticated synthetic summary
look like a valid provider run. This was locally attractive because execution
preflight appeared to own authentication, while the aggregate evidence consumer
paid the provenance gap. Classification: `boundary inversion`, severity
E3/A1/F2/K3/T2 = 11, confidence C3. Aggregation now independently requires the
policy's allowed non-API session class, exact policy pin, proof that auth probe
output was not retained, and a valid provider-host attestation. Missing or
substituted HMAC evidence and a completed-looking caller-only assertion are
release failures.

The first release workflow optimized job count by building in the authenticated
provider job and directly interpolating dispatch inputs into shell text. That
made the session-bearing runner absorb repository/dependency execution and
shell-parsing risk. Classification: `boundary inversion`, severity
E3/A2/F2/K3/T2 = 12, confidence C3. A hosted prepare job now creates the
short-lived bundle; SHA-pinned artifact actions are the only external action
code on the provider lane, provider-label pairing is checked structurally, and
unsafe input/path mutations have negative conformance tests.
Provider execution is additionally restricted to `refs/heads/main` and names a
provider-specific GitHub Environment; the hosted prepare job may run elsewhere,
but a non-main ref cannot reach the session-bearing job. Actual Environment
reviewer/branch protection remains external configuration and is not inferred
from the workflow string.

An early real Claude run requested a model alias that the CLI resolved to a
different canonical model. Treating the request string as the executed model
would optimize a green matrix at the cost of reproducibility. Classification:
`temporal`, severity E2/A1/F2/K2/T2 = 9, confidence C3. The harness now records
reported model identities and fails an observable mismatch; the final run must
request the reported canonical ID.

One real Codex design serialized the two requested writers but introduced
analysis nodes whose live-file reads could overlap the first write. Optimizing
the visible writer/writer edge left a hidden read/write conflict.
Classification: `boundary inversion`, severity E2/A1/F2/K2/T1 = 8, confidence
C3. The design Skill now requires ordering every conflicting resource pair and
recommends a read barrier or immutable snapshot; the canonical linter remains
the decision owner.

Promoting the three safety conditions from manual-only review to deterministic
fields is locally attractive because it removes reviewer variance. Keeping the
same exact condition in the manual checklist would merely shift duplicated work
to release reviewers. Classification: `externalization`, severity
E1/A1/F1/K2/T1 = 6, confidence C2. Deterministic assertions now own the exact
action/retry fields; manual review is limited to non-fabrication and rationale.

## Counterfactuals

- A — Keep per-provider summaries and rely on a release reviewer to find
  omissions: small implementation, large compensation halo. Rejected.
- B — Add only matrix counting: catches missing rows but not substituted files,
  stale reviews, cost authority, or evidence provenance. Rejected.
- C — Validate exact retained evidence, bind independent decisions to run
  hashes and limits, require broker-signed run/host CLI-session provenance,
  retain content-addressed reports, and keep all outputs unaccepted until
  review. Adopted.

## False positives and residual risks

- [Evidence] Similar request/response logic in the runtime pilot and independent
  MCP client is intentional implementation independence, not accidental rule
  duplication; neither owns CaseGraphen decision semantics.
- [Evidence] The aggregate workflow not passing without an external manual
  review is the intended trust boundary, not a CI availability defect.
- [Hypothesis] Real authenticated Codex and Claude CLI sessions may reveal
  provider-specific output, session-expiry, or cost-telemetry differences. No
  API key was used. An interrupted CLI-session evaluation is not retained or
  treated as release evidence; promotion still requires broker attestation.
- [Hypothesis] Model comparison applies only when a recognized provider envelope
  emits a non-empty top-level model observation. `observable: false` is retained
  as absence, not evidence that the requested model executed; provider-specific
  nested envelope formats still need real-run calibration.
- [Hypothesis] HMAC verification proves possession of the configured broker
  key and run binding, not that deployed OS accounts, key ACLs, or credential
  brokers actually prevent agent reads. That remains externally auditable host
  provisioning and must not be inferred from repository tests.
- [Hypothesis] Four local families do not establish production durability,
  remote transport security, or performance under load. Promotion remains
  false in the retained report.

Within the issue boundary, no material local optimum remains after widening
the evaluation boundary from individual runner success to exact retained
matrix evidence, independent review authority, and post-run provenance. This
is not a claim of a global optimum or stable-contract readiness.

# Issue #57 Graph Engineering hardening local-optima audit

## Scope and outcome

- Scope: the twelve-child stabilization program covering content/revision
  binding, expansion bounds, streaming permits, anchor provenance, schemas,
  Skills/evals/docs, product surface, runtime pilots, MCP operations and MSRV CI.
- System outcome: experimental Graph Engineering contracts can begin real
  runtime integration without being represented as stable or bypassing the
  acceptance ledger.
- Evidence: child audits #58–#69, two real-provider smoke runs, two local
  runtime pilots, product/schema/Skill conformance, and the full Rust 1.80
  static-analysis gate.
- Verdict: implementing each review item as an isolated patch would have been a
  **cross-layer local optimum**. The completed program binds the review,
  runtime, resource, product, documentation and release boundaries together.

## Evaluation conditions

| Variable | Narrow completion condition | Expanded program condition |
|---|---|---|
| `B` | one experimental Rust module | case ledger, topology, runtime, host, Skills, schemas and release operations |
| `M` | feature/tests compile | exact authority binding, fail-closed reconciliation, usability and observable promotion evidence |
| `N` | edit the reported function | canonical owners plus adapters, inventories, docs and negative tests |
| `T` | current v0 implementation | runtime pilots, provider drift, restart, stale revision and future promotion |

## Facts, inferences, and hypotheses

- **Observed:** topology review authority is now bound to topology ID/hash,
  artifact, claim, case space and observed revision; edits invalidate authority.
- **Observed:** expansion counts actual canonical node additions, streaming
  permits bind exact revision/attempt/resources, and tool-observed anchors are
  opaque and provenance-bound.
- **Observed:** 27 experimental contracts and four Skills are inventoried and
  fail closed under mutation fixtures; eight product workflows delegate to
  canonical modules through a durable authenticated host.
- **Observed:** runtime pilots exercise JSONL and isolated Git worktrees, while
  Codex and Claude smoke runs preserve the independent review seam.
- **Observed:** all repository gates pass under the declared Rust 1.80 MSRV.
- **Inference:** the main remaining risk is operational vocabulary learning from
  more runtime integrations, not a known authority shortcut. Keeping v0
  experimental makes that learning reversible.
- **Hypothesis:** tool-specific typed MCP payload schemas will be needed before
  multiple independent clients can rely on this surface as stable.

## Candidate ranking

| Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---|---|---|---|---:|---|---|
| stabilize after architecture-only implementation | quick stable label | freezes unbound reviews, proposal-count budgets and stale permits | product lifecycle | 14/15 | C3 | rejected systemic optimum |
| move runtime decisions into the core | fewer integration seams | duplicates scheduler/model/retry ownership and weakens ledger boundary | architecture | 12/15 | C3 | rejected displaced-cost optimum |
| harden experimental plane before pilots (implemented) | more contracts/tests/docs now | modest maintenance and migration cost | system/lifecycle | 6/15 | C3 | selected |

The former compensation halo was host-specific code, manual schema/Skill
review, ad-hoc runtime completeness checks, and operator recovery. Its cost
would have appeared only after deployment, making the locally smaller design
more expensive at the product boundary.

## Boundary inversion and migration valley

- At the function level, explicit bindings, inventories and opaque permits add
  types and checks.
- At the subsystem level, canonical ownership prevents the CLI, MCP host,
  Skill and runtime adapter from implementing competing truth rules.
- At the product level, one supported surface and durable refusal semantics
  replace adopter-specific glue.
- Across releases, retaining `experimental v0` preserves the option to revise
  contracts after more pilots instead of converting current examples into
  permanent compatibility debt.

The migration valley is bounded: adopters must supply explicit hashes,
revisions, typed patches and auth, and existing v0 payloads may break. That cost
is intentionally paid before stable promotion.

## Score and decision

- Premature stabilization: `E=3`, `A=3`, `F=3`, `K=2`, `T=3`; severity
  `14/15`, confidence `C3`.
- Classification: systemic and time-delayed.
- Decision: close the hardening program as implemented, but do **not** promote
  v0 automatically. A follow-up promotion review must choose either a revised
  v0 based on additional runtime evidence or an explicitly reviewed stable
  proposal.

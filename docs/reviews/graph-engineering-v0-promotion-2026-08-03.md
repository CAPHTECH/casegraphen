# Graph Engineering v0 promotion review — 2026-08-03

## Decision

**Retain and revise experimental v0; do not propose a stable contract yet.**

The integrity blockers reported against commit `092fdcda49066be243b76a651c395daf47db3c05`
are resolved and the plane is suitable for additional runtime-integration
pilots. The available evidence is not broad enough to freeze the vocabulary as
stable.

## Evidence reviewed

- Content- and revision-bound topology reviews, streaming permits and typed
  expansion diffs, with substitution/replay negative tests.
- Opaque tool-observed anchor proofs separated from caller declarations.
- Two real local runtime adapters: generic JSONL subprocess and isolated Git
  worktree integration, including retry/missing/schema/resource failure cases.
- One real Codex and one real Claude fresh-agent smoke preserving the evidence
  review seam. This is not the full 20-run release matrix.
- Operational authenticated/durable MCP host and a supported eight-workflow
  product surface delegating to canonical CaseGraphen modules.
- Rust/schema/Skill/product conformance and all tests passing under Rust 1.80.

## Why stable promotion is deferred

1. Runtime evidence covers two local adapter shapes, not multiple production
   schedulers or long-lived fleet deployments.
2. The full Codex/Claude ten-scenario matrix and its manual judgments have not
   been completed as a promotion report.
3. The operational MCP host currently uses shared JSON payload envelopes;
   tool-specific stable payload schemas should be evaluated with independent
   clients before compatibility is promised.
4. Active/active host persistence and external identity-provider integration
   remain intentionally unsupported.

## Next review trigger

Revisit promotion after at least two additional runtime families, a completed
fresh-agent release matrix, and one independent MCP client have produced
retained reports. At that point, either publish a reviewed stable proposal or
revise v0 again. No runtime report or passing test automatically performs that
decision.

# Graph Engineering v0 promotion review — 2026-08-03

## Decision

**Retain and revise experimental v0; do not propose a stable contract yet.**

Topology review authority now includes the canonical deployment-policy
manifest hash as well as topology bytes and the observed case revision. Stable
promotion requires preserving this boundary: compilation must reproduce the
reviewed manifest, and topology/policy changes must obtain a new review.

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
- A later local CLI-session matrix executed all ten scenarios for Codex and
  Claude with 20/20 deterministic and 20/20 independent manual passes. It is
  explicitly non-promotion evidence because the workstation runs lack
  privileged provider-host attestations; see
  [`2026-08-04-cli-session-matrix.md`](../evals/fresh-agent/2026-08-04-cli-session-matrix.md).
- Operational authenticated/durable MCP host and a supported eight-workflow
  product surface delegating to canonical CaseGraphen modules.
- Rust/schema/Skill/product conformance and all tests passing under Rust 1.80.

## Why stable promotion is deferred

1. Runtime evidence covers two local adapter shapes, not multiple production
   schedulers or long-lived fleet deployments.
2. The full Codex/Claude ten-scenario matrix and its manual judgments pass
   locally, but have not been produced by audited provider-specific runners
   with broker-signed host/session provenance and durable workflow artifacts.
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

Issue #76 adds reproducible evidence infrastructure for this trigger: a strict
two-provider/ten-scenario aggregator, four materially distinct local runtime
families (including two resource-bearing generic-JSONL families), and a
stdlib-only Python MCP client that reaches the topology-to-review seam. The
local real-provider behavior now passes, but the release aggregator correctly
refuses promotion on the two missing host attestations. Stable promotion still
requires the same matrix on provisioned provider-specific runners,
broker-signed provenance, and retained workflow artifacts.

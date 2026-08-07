# Graph Engineering v0 promotion review — refreshed 2026-08-05

## Decision

**Retain and revise experimental v0; do not propose a stable contract yet.**

```text
promotion_recommended: false
```

The authority chain now binds accepted topology and policy bytes to reviewed
compilation, reviewed resource reservation, canonical terminal attempts, and
byte-observed data-edge handoffs. These are substantial local results. They do
not replace provider-host authority, retained external evidence, sustained
allocator evidence, or a production fleet observation.

The machine-readable source for counts, retained references, completed local
triggers, and open blockers is
[`graph-engineering-v0-promotion.inventory.json`](graph-engineering-v0-promotion.inventory.json).
Conformance rejects a trigger that remains listed as open after all of its
facts have retained evidence. As of 2026-08-05 the supported product surface is
11 workflows, four local runtime families, and experimental contract v0. The
inventory records exact evidence dates, evaluated commits, and content hashes;
this prose does not redefine those decision rules.

## Completed retained evidence

<!-- promotion-trigger:runtime-families -->

- Four materially different local runtime families have retained pilot
  evidence: generic JSONL subprocess, isolated Git worktree, SQLite durable
  queue, and asyncio event stream.

<!-- promotion-trigger:local-provider-matrix -->

- The Codex CLI and Claude Code authenticated-session matrix completed all ten
  scenarios for both providers. It is explicitly non-promotional because the
  workstation executions lack external host/session authority and signed
  independent review.

<!-- promotion-trigger:independent-client -->

- The stdlib-only independent MCP client reaches `needs_review` with
  `accepted: false` and does not depend on the Rust client implementation.

<!-- promotion-trigger:edge-completeness -->

- Runtime reconciliation derives node and data-edge expectations from the
  reviewed topology and requires byte-observed terminal-attempt handoffs.

<!-- promotion-trigger:reviewed-compilation -->

- The reviewed compile and resource-authority path is exercised by retained
  local durability evidence while preserving the final evidence-review seam.

<!-- promotion-trigger:bounded-durability -->

- Bounded pilots cover a separate-process TCP boundary, non-UTF-8 artifacts,
  a 512-node/511-edge/128-retry run, and 512 allocator journal events. This is
  a bounded local threshold, not a sustained-fleet claim.

<!-- promotion-trigger:durable-runtime-evidence -->

- Issue #89 published a deterministic content-addressed package through a
  required-reviewer environment at exact commit `a8d11a9...`, re-downloaded
  the Release asset, verified its SHA-256 and internal inventory in the
  workflow, and repeated strict offline verification independently. The small
  exact retained record is checked in under `docs/pilots/issue-89`. This is
  durable non-promotional evidence, not WORM storage or CaseGraphen acceptance.

<!-- promotion-trigger:verification-product-surface -->

- Issue #87 exposes verification-lineage proof derivation as a supported
  product workflow and documents the ordinary CLI-to-proof authority path.

<!-- promotion-trigger:compiler-profile-compatibility -->

- Issue #91 binds compiler identity and input-contract versions into topology
  review and deployment bundles, retains exact profile-0 compatibility,
  refuses identity substitution, emits review-required migration proposals,
  and gates representative verification cost. This is repository lifecycle
  evidence, not multi-release production history.

<!-- promotion-trigger:allocator-fleet-replay -->

- Issue #88 retained the required 10k and 100k journal-scale reports —
  checkpoint creation/verification/compaction, restart replay, and
  reserve/release latency — from clean source revision `9b23383...`, passing
  every threshold ADR 0026 defines as its release-evidence lanes. This is
  retained release-candidate evidence, not sustained-fleet operation or
  promotion authority.

All completed items above remain `accepted: false` unless and until an
independent acceptance-ledger review records a separate decision.

## Required stable-promotion blockers

<!-- promotion-trigger:provider-authority -->

1. **Provider-specific evaluation authority — Issue #76.** Retain exact
   provider-runner host/session proofs, broker countersignatures, signed
   independent review, and final durable evidence produced by the protected
   workflow. Authenticated Codex CLI and Claude Code sessions are required;
   provider API keys are not part of this lifecycle.

<!-- promotion-trigger:production-fleet -->

2. **Production or remote fleet evidence — Issue #116.** Retain evidence from
   a real long-lived scheduler/fleet boundary, including remote failures and
   recovery. Issue #89 built the content-addressed retention mechanism this
   evidence would use, but supplied none of the evidence itself. Local
   adapter diversity and bounded synthetic scale still do not prove this
   behavior.

These blockers are conjunctive. Satisfying one does not weaken another, and a
runtime report or passing test never performs promotion automatically.

## Optional post-v0 enhancements

The following are useful but are not disguised stable-promotion blockers:

- tool-specific stable MCP payload schemas;
- active/active operational-host persistence;
- external identity-provider integration;
- WORM or transparency-log mirroring beyond content-addressed GitHub Release
  evidence.

## Evidence retention and review rule

Issue #89 makes future runtime-durability evidence a deterministic,
content-addressed GitHub Release asset. Publication must use the same protected
trusted-source and exact-SHA provenance principles as the fresh-agent release
pipeline, reject conflicting pre-existing assets, re-download the published
bytes, and run offline verification. Ordinary CI uses bounded synthetic
fixtures and must not download historical evidence packages.

GitHub Releases remain administratively mutable and are not a WORM guarantee.
The retained repository record therefore identifies the exact tag, asset,
byte length, SHA-256, evaluated commit, workflow run/attempt, topology hash,
and reviewed deployment hash. A later review must verify that record and asset
rather than treating a release URL as authority.

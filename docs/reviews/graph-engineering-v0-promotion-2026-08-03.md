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
- Four materially distinct local runtime families: generic JSONL subprocess,
  isolated Git worktree integration, SQLite durable queue, and asyncio event
  stream. The regenerated report proves canonical terminal-attempt handoffs
  for every data edge, rather than node success alone.
- A later local CLI-session matrix executed all ten scenarios for Codex and
  Claude with 20/20 deterministic and 20/20 local qualitative passes. The two
  review records are intentionally unsigned and are not accepted as release
  authority. This is explicitly non-promotion evidence because the workstation
  runs lack evaluation-host proofs, provider-broker countersignatures, and
  signed reviewer identity; see
  [`2026-08-04-cli-session-matrix.md`](../evals/fresh-agent/2026-08-04-cli-session-matrix.md).
- Operational authenticated/durable MCP host and a supported ten-workflow
  product surface delegating to canonical CaseGraphen modules.
- The stdlib-only independent MCP client now has a checked-in deterministic
  report in the Issue #76 retention manifest; it reaches `needs_review` with
  `accepted: false` and does not rely on custom Rust client code.
- Bounded durability pilots cross a separate-process TCP boundary, retain a
  non-UTF-8 binary handoff, reconcile a 512-node/511-edge/128-retry matrix, and
  replay 512 real allocator journal events. See
  [`issue-85/README.md`](../pilots/issue-85/README.md). Every lane passes its
  local threshold while remaining `accepted: false`.
- Rust/schema/Skill/product conformance and all tests passing under Rust 1.80.

## Why stable promotion is deferred

1. Runtime evidence covers four local adapter shapes plus bounded durability
   lanes, not production schedulers or long-lived remote fleet deployments.
2. The full Codex/Claude ten-scenario matrix and its qualitative judgments pass
   locally, but have not been produced by audited provider-specific runners
   with evaluation-runner host/session proofs, provider-broker
   countersignatures, and signed independent-review authority. The repository
   now encodes evaluation, provider-specific
   evaluation-runner Ed25519 host/session proof, provider-broker
   countersignature, signed manual review, strict public-key-only finalization,
   and content-addressed durable publication as separate workflows. The host
   proof—not a later broker CLI probe—binds the actual runner, run/attempt,
   workflow/head, artifact ID/digest, summary hash/challenge, auth class, and
   credential-isolation declaration. GitHub still has no registered
   self-hosted provider/broker runners or protected environments, so that
   lifecycle has not produced promotion evidence.
3. The operational MCP host currently uses shared JSON payload envelopes;
   tool-specific stable payload schemas should be evaluated with independent
   clients before compatibility is promised.
4. Active/active host persistence and external identity-provider integration
   remain intentionally unsupported.
5. The allocator still performs O(event-count) full replay and has no canonical
   checkpoint/compaction contract. The 512-event pilot is passing evidence for
   the bounded threshold, not a sustained-fleet claim.

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
independent client report is retained, and the complete evidence lifecycle is
conformance-gated rather than left as an operator-assembled command. Local
real-provider behavior passes, but the release aggregator correctly refuses
promotion without the two host attestations and cryptographically identified
reviewer. Stable promotion still requires the same matrix on provisioned
provider-specific runners, external evaluation-host proof, broker-signed
provenance, signed independent review, and verified durable final workflow
artifacts. Every privileged job must run workflow YAML at the exact protected
trusted-verifier SHA and execute verifier helpers from that same SHA.

Issue #85 adds graph/runtime durability evidence without taking ownership of
that provider attestation gate. Its remote, binary, scale, and allocator lanes
all fail closed at the review seam. Stable promotion remains blocked until #76
has evaluation-host-signed and provider-broker-countersigned session
provenance, regardless of these local durability results or the current
ten-workflow product surface.

The retained promotion artifact must be independently re-verifiable after
protected-variable rotation. The current package therefore includes the public
PEM/key-provenance records, release policy, evaluator baseline, scenario
manifest, evaluation-host proofs and public keys, broker attestations, signed
review, workflow provenance, and trusted-source inventory. GitHub Releases are
content-addressed and re-downloaded but remain administratively mutable; this is
durable review evidence, not a WORM guarantee. No provider API key participates
in this lifecycle: Codex and Claude Code use authenticated CLI sessions only.

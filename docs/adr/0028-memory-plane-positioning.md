# ADR 0028: The Memory Plane is a governed derived view

- Status: Accepted for experimental v0
- Date: 2026-08-06
- Parent issue: [#92](https://github.com/CAPHTECH/casegraphen/issues/92)

## Context

Long-running agents need reusable project context, but retaining text or an
embedding does not establish that the content is current, authorized, or even
grounded in an observation. Summarization can erase the distinction between a
user requirement, external material, tool output, and an agent inference. A
search rank can then be mistaken for authority, allowing stale or low-authority
content to influence later sessions.

ADR 0002 positions CaseGraphen as the Acceptance Ledger rather than the agent
runtime and records past-judgment learning as out of scope for the core. That
boundary remains correct. What is missing is an optional surface that can
structure evidence-backed memory candidates and derive reusable context from
already accepted state without making the ledger learn or granting the runtime
a new write path.

## Decision

1. Memory Plane is an **experimental, optional product surface**. Core remains
   the Acceptance Ledger and does not execute models, schedule agents, or learn
   autonomously.
2. Accepted Memory is a derived Case Graph view. The MorphismLog, replayed
   CaseSpace, and content-addressed artifacts remain the only durable source of
   truth. A lexical or vector index is replaceable and non-authoritative.
3. Source Record, Memory Claim, Query, Projection, Use Report, Policy, and
   Index are separate v0 contracts. An immutable source exists before
   extraction; a claim cannot contain `accepted` or caller-declared trust.
4. All proposed claims enter as `evidence`, lifecycle `proposed`, boundary
   `inferred`, and review status `unreviewed`. The proposal API performs no
   mutation. Acceptance, rejection, and application continue through the
   existing review and operation-gate seams.
5. Retrieval is a read-only projection bound to one exact replayed revision,
   requesting actor, audience, purpose, scope, valid-time cutoff, risk class,
   memory kinds, and budget. Scope, authority, sensitivity, valid time, and
   conflict filters run before relevance ranking. Omissions and budget loss are
   part of the result and its content hash.
6. v0 uses the following authority lattice:

   ```text
   untrusted < observation < project_fact < project_constraint < project_authority
   ```

   A derivation cannot exceed the lower of its Source Record origin ceiling and
   provenance-role ceiling. Elevation requires a separate hard
   `authorized_by` relation accepted by a suitably authoritative reviewer.
   Confidence and repetition do not raise authority. The v0 memory proposal
   helpers do not mint this elevation: an elevated claim and its binding must
   be authored and reviewed through the canonical morphism path, after which
   replayed queries validate the accepted binding.
7. v0 is bitemporal. Transaction time is the selected MorphismLog revision;
   valid time is the claim's `[valid_from, valid_until)` interval. A current
   query returns only current accepted claims. Explicit historical queries may
   expose candidate, rejected, superseded, expired, retracted, and not-yet-valid
   claims with their status preserved.
8. An accepted contradiction makes both claims `contested`. An unresolved hard
   contradiction is excluded from normal current projections. Contested state
   is returned only when explicitly requested and remains visible in projection
   metadata even when omitted.
9. Canonical human statements are not an exception to immutable capture in v0.
   The statement is first retained as a Source Record; its role may permit the
   highest ceiling, but its bytes, actor, boundary, and review remain auditable.
10. The first pilot is project memory for coding agents. Personal life-log,
    sensitive-person inference, cross-organization sharing, autonomous review,
    and parametric memory are excluded.

## Invariants

- No accepted memory without an immutable reachable source.
- No authority amplification without an independently accepted authority
  binding.
- No caller field can create acceptance.
- No projection or index can change managed state.
- No expired claim is returned as current.
- No hard conflict is hidden.
- Deleting every index leaves accepted memory reconstructible from replay and
  artifacts.
- A Memory Use Report is an untrusted runtime self-report, not evidence that the
  agent obeyed the projection.

## Relationship to ADR 0002

This ADR narrows the earlier non-goal; it does not reverse the runtime boundary:

> CaseGraphen core does not autonomously learn from past judgments. The optional
> Memory Plane may propose evidence-backed, reviewable claims about prior
> outcomes without bypassing the Acceptance Ledger.

## Consequences

- Read paths can ship before new durable write paths because existing accepted
  evidence cells can be projected when they carry the experimental claim
  metadata.
- Review load remains explicit. v0 performs no automatic acceptance, even for
  low-risk claims; deterministic auto-promotion would require a later ADR with
  a closed transformation vocabulary.
- Transaction-time history is requested by replaying the desired revision and
  binding that exact revision in the query. The Memory Plane does not maintain
  a second historical store.
- Vector search may be supplied by an adapter after authorization filters, but
  cannot become a source of truth or silently widen the candidate set.
- Stable promotion requires retained evidence for laundering, stale-memory,
  hard-conflict, replay, poisoning, and retrieval-quality criteria.

## Rejected alternatives

- A memory product or vector database as the canonical store: splits review,
  authority, and replay truth.
- Direct `memory remember`, `write`, `accept`, or `forget` operations: collapse
  proposal, acceptance, and invalidation into an unauditable command.
- Agent-managed acceptance: lets a model approve its own inference.
- Embedding similarity as trust: confuses relevance with authority and time.

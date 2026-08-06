# ADR 0031: GitHub issue-to-PR evidence is bounded, replayable observation

- Status: Accepted for experimental v0
- Date: 2026-08-06
- Parent issue: [#102](https://github.com/CAPHTECH/casegraphen/issues/102)
- Design: [issue-102-github-evidence-adapter.md](../design/issue-102-github-evidence-adapter.md)

## Context

Dogfooding issue #92 through PR #101 required manually translating GitHub
issues, PR head/base identity, CI conclusions, review findings, and thread
resolution into review evidence, and manually separating implementation-agent
self-review from independent review. The manual process was auditable but
inconsistent, and it exposed a laundering hazard: a verification record can
remove an obstruction procedurally even when the verification was only
self-review, unless independence and source authority stay explicit.

ADR 0028 established the pattern for an experimental evidence surface: an
immutable source exists before extraction, caller fields cannot create
acceptance, and projections are read-only and loss-explicit. The verification
lineage work established that different actor ids never prove independent
minds (`independent_minds_not_observable`). Issue #102 extends both to
provider observations from GitHub.

## Decision

1. The GitHub evidence adapter is an **experimental, optional, store-free
   product surface**. Its commands (`github observe|refresh|project`) open no
   CaseStore, append nothing, and emit records that carry `accepted: false`
   and `mutation_performed: false` as schema constants. GitHub, CI, review
   bots, and the implementation agent are observation sources, never
   acceptance authorities.
2. Provider data enters only as **content-addressed capture**: a
   caller-authored `github.capture_manifest.v0` (strict-parsed; no trust
   vocabulary exists in it) naming raw `gh` output files whose bytes are
   bound by SHA-256 and retained as `memory.source_record.v0` entries with
   `authority_origin: "tool"`. The existing memory authority lattice
   therefore caps everything derived from GitHub at `observation` authority;
   this ADR mints no new authority rule.
3. Normalization is **deterministic and replayable**: its only inputs are the
   manifest bytes and artifact bytes — no clock, environment, network, or
   store. Identifiers are content-derived, orderings are total and stated,
   and every record's `sha256:` content hash is computed over its canonical
   serialization via the one existing hash implementation. Rebuilding from
   the retained artifacts reproduces byte-equivalent normalized evidence and
   projection hashes.
4. Every PR review snapshot binds the **exact repository, PR number, base
   SHA, and head SHA**. A refresh compares against the previously observed
   head and reports `head_unchanged` (with explicit same-head drift such as
   disappearing checks and edited comments) or `stale_head`.
   `review_basis_moved` is a schema constant `false`: a refresh cannot
   rebase; moving the basis requires a new observation with a visibly
   different identity and hash. The declared previous observation is the one
   record accepted back as input; its content hash is recomputed on load and
   a mismatch refused — which proves self-consistency, not tool provenance:
   the operator, not the file, is what vouches for the chosen basis.
5. **Review independence is a computed class, never a caller claim and never
   a proof.** Actor identity is the GitHub node id — logins are renameable
   display strings, and the retained corpus shows one actor under two
   logins (`coderabbitai` / `coderabbitai[bot]`, both `BOT_kgDOCCSy2w`) —
   and the implementation actor set is derived by comparing node ids
   within the same snapshot: the captured PR author plus commit authors
   and committers, never a caller-supplied flag. An artifact that must
   feed the actor set but lacks node ids is refused; login matching is not
   a fallback. Bot identity is a provider attestation from an ordered,
   closed list: the GraphQL Actor discriminator (`__typename` other than
   `User` — `Bot`, `Organization`, `Mannequin`, … all fail closed into the
   non-human role), the provider-issued `BOT_` node-id prefix, and id
   equality with an actor already attested in the same capture. The
   id-keyed rules are load-bearing on the real corpus, not defensive
   extras: the provider attests the same id `BOT_kgDOCCSy2w` as `Bot` on
   its comments and `User` in `resolvedBy` within one capture, so
   per-occurrence typename attestation is self-contradictory on real data —
   a bot attestation is sticky per actor id, and a `User` typename never
   overrides it. Absent all three, the actor is `unattributed`: absence is
   recorded, never resolved by a name heuristic, and an unattributed actor
   satisfies nothing. Classification is total
   and closed — `self_review`, `automated_bot`, `ci_check`,
   `independent_human_candidate`, `unattributed` — with the
   implementation-actor arm first, so an implementation actor or a bot
   structurally cannot classify as an independent candidate, and
   `authorAssociation` (e.g. `MEMBER`) is never an input. A policy
   requiring independent review is satisfiable only by an `APPROVED`
   review from a provider-attested `User` outside the implementation actor
   set whose review `commit.oid` equals the observed head — an approval
   without that exact binding is excluded and the exclusion recorded, not
   credited by fallback. Self-review, bot status, CI success, and
   unattributed actors are type-unable to satisfy the policy.
   `independence_proven` is a schema constant `false`, and every
   independence record carries the `independent_minds_not_observable`
   finding: a candidate is a candidate, not proven independence.
6. The **compact reviewer projection declares its loss**. Must/Should/Can
   tiers, blocking and non-blocking findings, unresolved threads, failed
   checks, verification sources with independence class, and residual risks
   are derived by a single deterministic rule. Checks are three-way: failed,
   successful, or **inconclusive** (`NEUTRAL`/`SKIPPED`/`CANCELLED`/still
   running) — an inconclusive check is not a failure but is not evidence
   either, so it surfaces in Should Review and as a `checks_inconclusive`
   residual risk instead of disappearing into success. Omitted content
   (hashed bodies, unmapped provider fields, provider truncation) is
   declared in `losses`, and `full_trace` keeps the complete audit graph
   separately reachable. A projection that cannot show its gaps is not a
   valid projection.
7. All mutation-capable follow-ups stay behind the existing seams. Attaching
   adapter output to a case space uses the gated `evidence attach`; its
   acceptability is decided by the unchanged `evidence_trust` rule; review
   and transition flow through the canonical review morphisms and operation
   gates. This surface adds no mutation path and no gate exemption.

## Invariants

- No caller field can create acceptance, approval, authority, or
  independence.
- No observation derived from GitHub exceeds `observation` authority without
  the existing, separately reviewed elevation path.
- No refresh moves a review basis; a stale head is an explicit report.
- No implementation actor's review, bot finding, CI conclusion, or provider
  role/association satisfies an independent-review requirement; no approval
  counts without an exact observed-head commit binding.
- No actor gains a human or bot class without a provider attestation from
  the closed list; an unattested actor is `unattributed`, and an artifact
  that cannot supply the actor-id set is refused, not patched by login
  matching.
- No provider state observation is coerced: `mergeable` stays the
  three-state provider value (`MERGEABLE`/`CONFLICTING`/`UNKNOWN` — a merged
  PR legitimately reports `UNKNOWN`), never a boolean.
- No hidden loss: hashed-out content, unmapped provider fields, and provider
  truncation are declared in the projection.
- Deleting every derived record leaves the normalized evidence and
  projection reconstructible, byte-equivalent, from the retained manifest
  and source artifacts.
- No credentials, authorization headers, or raw environment data enter a
  retained artifact.

## Consequences

- The Issue #92 / PR #101 corpus is retained as the first pilot: two
  successful Quality checks, nine resolved actionable review threads, the
  exact final head/base, and the absence of independent human approval are
  reproducible offline without laundering any of them into acceptance.
- Reviewer effort shifts from collecting provider state to judging it: the
  projection can say "no blocking findings" while still declaring the
  rate-limited bot review and the self-review limitation as residual risks.
- The adapter cannot verify that a GitHub account is a distinct mind;
  `independent_human_candidate` is the ceiling of what provider data can
  say, and policies that need more must bind reviewer identity through the
  existing capability and review seams.
- Stable promotion follows the ADR 0029 pattern: these contracts stay
  experimental v0, may change incompatibly with synchronized updates, and no
  automatic acceptance is authorized while they do.

## Rejected alternatives

- Extending `github_issue_snapshot.v1` (the lift input): conflates "what the
  case is about" with "what was observed about the work" and would push
  evidence semantics into a stable lift contract.
- Live provider access from the tool: makes evidence unreproducible and
  drags credentials into scope; capture stays outside, in `gh`, run by the
  operator.
- A caller-suppliable independence file or `--implementation-actors` flag:
  reopens the laundering hole the classification exists to close.
- Mapping provider states to booleans (`merged: true`): destroys the
  observation record; verbatim provider strings are the evidence.
- Auto-attaching observations as evidence cells: would make the adapter a
  mutation path and GitHub a de facto source of truth.

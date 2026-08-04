# Issue 91 implementation local-optima audit

Date: 2026-08-05

Mode: `deep-dive` with intervention-oriented counterfactuals.

Snapshot: the shared working tree based on
`d353b160ebcca089c47d5df455c9357106389989`. The Issue #91 implementation is
not yet represented by one clean commit, so this audit refers to the observed
working-tree contents rather than claiming a commit-level result.

## Scope and available evidence

The audit covers compiler profiles 0 and 1, topology-review compiler binding,
retained compiler inputs, semantic bundle verification, migration proposals,
the bounded verification gate, schema and documentation inventories, and the
historical-profile compatibility tests.

Observed runtime evidence:

- `cargo test graph_compiler --lib`: 14/14 passed.
- The pre-profile review replay test passed and assigned missing compiler
  fields to profile 0.
- `cargo clippy --lib -- -D warnings` and `cargo fmt --all --check` passed.
- Experimental schema conformance passed for 41 governed contracts.
- Promotion-review conformance passed with seven completed local triggers and
  five open promotion blockers.
- The checked-in compiler performance report passed source/workload-aware
  verification. Rehashed over-budget, workload-substitution, and
  compiler-source-substitution fixtures were all refused.
- A fresh current-binary pilot passed at 4, 128, and 512 nodes. It observed
  12, 384, and 1,536 policy documents and approximately 16, 330, and 1,376 ms
  wall time respectively on this host.

Limitations: no production topology distribution, multi-release profile
history, 10k-node run, cache, p95/p99 series, or remote fleet evidence was
available. The current benchmark creates 2, 64, and 256 typed data edges for
the 4/128/512-node cases; the report contract does not retain edge count.

## System outcome and evaluation conditions

The desired system outcome is that a reviewed deployment remains reproducible
under its exact compiler semantics for the lifetime of its authority, while
unknown semantics fail closed and verification remains operationally bounded.

| Axis | Current evaluation condition | Expanded condition used here |
|---|---|---|
| `B` boundary | compiler module and one bundle | review ledger, bundle authority, CI evidence, migration, and release lifecycle |
| `M` metric | exact byte reproduction | authority isolation, historical recovery, refusal strength, evidence freshness, latency, memory, and change amplification |
| `N` change scope | compiler dispatch and schemas | review target, retained corpus, migration contract, benchmark, CI, ADR, and product inventory |
| `T` time | current experimental release | repeated semantic-profile upgrades and historical retention |

Constraints are fail-closed authority, offline verification, deterministic
replay, experimental-v0 compatibility, and no cache-derived authority.

## Executive verdict

The authority and compatibility design is a **global improvement within the
audited experimental-v0 boundary**, not a harmful local optimum. Review
authority now binds the compiler version, semantic profile, retained input
schema, and complete contract-inventory hash. Pre-profile reviews replay as
profile 0, and a profile-0 review cannot authorize a profile-1 compile. Bundle
verification dispatches exact schema/version pairs and performs one full
recompile plus byte-for-byte bundle comparison.

Issue #91's ten acceptance criteria are met within the audited experimental-v0
boundary. Profile 0 and profile 1 still share lowering code. Three
complete-manifest content addresses detect drift across base, alternate, and
reviewed modes, but the retained corpus is encoded as hash assertions rather
than retained bundle bytes. This is a `time-delayed` maintenance candidate,
not a current authority failure or an unmet Issue #91 criterion.

The previous performance-report externalization is closed. Verification now
binds the report to the current benchmark and compiler source, re-derives the
expected edge, policy, and artifact counts, re-derives each case result and the
overall gate, and refuses rehashed workload/source substitutions. The compiled
binary digest is deliberately named `observed_compiler_binary_sha256`; it is
evidence, not a reproducible source or authority claim.

## Acceptance criteria recheck

| # | Criterion | Result | Independent evidence / qualification |
|---:|---|---|---|
| 1 | Bind implementation/version and all semantic input contracts | Met | v1 inputs and manifest bind compiler identity and typed contract inventory; review target also binds version/profile/schema/inventory hash |
| 2 | Fail closed for same, historical, unknown, and future identities | Met | exact `(inputs schema, compiler version)` dispatch; manifest disagreement and future/unknown pairs refuse |
| 3 | Exact verifier for every supported identity | Met with lifecycle caveat | profile 0 has a separate strict input type and dispatch; three full-manifest hashes detect shared-lowering drift |
| 4 | Content-addressed, review-required migration proposal | Met | complete source/target identities, old/new bundle hashes, canonical identity differences, changed paths, `accepted:false`, and `requires_review:true`; rehashed identity substitution refuses |
| 5 | Historical fixture and negative substitutions | Met | base/alternate/reviewed profile-0 manifest corpus plus identity, contract, output, and future-version refusal tests |
| 6 | Representative small/medium/large benchmark | Met | current benchmark uses 4/128/512 nodes, typed data edges, three policy documents per node, latency/memory/artifact/input/recompile observations |
| 7 | Stable-promotion budgets and deterministic regression gate | Met | fixed thresholds and deterministic workload fields are re-derived, source hashes must match the checkout, negative rehash substitutions refuse, and static analysis runs the current benchmark |
| 8 | Cache is complete-keyed, replaceable, and non-authoritative | Met as non-applicable invariant | no cache exists; ADR retains full recompile as authority and recovery path |
| 9 | Inventory, docs, ADR, promotion, offline behavior | Met | governed schemas, ADR 0027, README/product surface, promotion review, and pilot guidance are connected by conformance checks |
| 10 | Full replay/recompile is canonical recovery | Met | verifier reconstructs the request from retained inputs, recompiles once, and compares complete artifacts and manifest |

## Ranked candidates

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Profiles 0/1 share lowering and retain only manifest content addresses | avoids duplicated decision rules | future semantic changes require careful extraction and cannot recover corpus bytes from hashes alone | lifecycle / third profile | 8 | C2 | `time-delayed` |
| 2 | Full semantic recompile on every verification | strongest compiler provenance | linear CPU/latency at every authority check | fleet/large topology | 5 | C2 | `harmless-locality` for current scale |
| 3 | Explicit manually enumerated contract inventory | reviewable complete identity today | new semantic dependency must be added across type/schema/example/tests/docs | repeated contract evolution | 7 | C2 | compensated `time-delayed` candidate |
| 4 | Proposal-only migration | prevents automatic authority transfer | operators cannot materialize migration without a separate future workflow | first real migration | 4 | C2 | intentional review seam |

## Detailed rationality cards

### Candidate 1: shared historical lowering

- **Owner / target:** compiler lifecycle maintainers and profile dispatch.
- **Original constraint:** profiles currently differ in identity/input envelope,
  not lowering semantics; duplicating all lowering decisions would create
  immediate rule divergence.
- **Local metric:** no duplicated compiler implementation and exact output
  hashes remain stable.
- **Current benefit:** strict v0/v1 parsing and exact dispatch preserve old
  semantics with little duplicated code.
- **Boundary cost:** a future semantic edit affects the historical path until
  it is extracted. Three manifest hashes detect this, but maintainers can still
  intentionally update them, and hashes alone are not an offline byte corpus.
- **Why local editing is insufficient:** the first real semantic divergence
  must add a new profile and freeze/extract prior lowering across code, tests,
  schemas, review identity, and retained evidence together.

### Candidate 2: full recompile

- **Owner / target:** semantic bundle verifier.
- **Original constraint:** self-consistent forged artifacts must not become a
  second authority source.
- **Local metric:** exact canonical reproduction and byte equality.
- **Current benefit:** current and historical bundles are verified from their
  retained inputs, with the invocation counter observing exactly one canonical
  compiler call.
- **Boundary cost:** verification cost grows with topology, edge, and policy
  size. The current 512-node/1,536-policy debug case is under budget, but no
  production percentile evidence exists.
- **Why local editing is insufficient:** a cache changes the authority seam and
  requires complete keys, corruption refusal, replacement, and canonical miss
  fallback across storage and operations.

## Compensation halo

```text
moving compiler semantics
  -> historical bytes could be reinterpreted
  -> strict v0/v1 input types + exact dispatch + review compiler binding
  -> compiler/review maintainers
  -> every semantic-profile change
```

```text
shared profile lowering
  -> an ordinary refactor can alter historical output
  -> three full-manifest content-address assertions + substitution tests
  -> compiler/test reviewers
  -> every lowering change
```

```text
full semantic recompile
  -> verification latency and memory move to the authority boundary
  -> 4/128/512-node current-binary benchmark + fixed CI budgets
  -> CI and release operators
  -> every quality-gate run
```

```text
human-readable performance report
  -> deterministic and observational fields coexist
  -> source hashes + workload re-derivation + explicitly observed binary/timing fields
  -> CI and promotion reviewers
  -> every current-binary and retained-report verification
```

All four compensations now state and enforce their respective boundaries. The
performance report does not turn binary or timing observation into compiler or
deployment authority.

## Boundary expansion and inversion

| Boundary | Current approach benefit | Current approach cost | Cross-boundary alternative | Alternative cost | Advantage |
|---|---|---|---|---|---|
| Function | exact dispatch/recompile is simple | one extra compile | trust manifest version | loses provenance | current |
| Module | one lowering owner prevents duplicated rules | profile coupling | separate profile modules now | immediate duplication | current |
| Feature | review and compiler semantics are content-bound | more review fields/contracts | version label only | silent reinterpretation | current |
| System | resource/deployment authority consumes verified bundle | full verification latency | cache hit as authority | second truth source | current |
| Operations | current CI measures the current binary and retained reports bind source/workload | host-dependent timing remains observational | require reproducible binary identity | toolchain/host attestation complexity | current |
| Lifecycle | exact profiles preserve old authority | retention/extraction burden grows per profile | immutable verifier/corpus registry | artifact signing and retention service | current for two profiles; reassess at several profiles |

Metric inversion tests:

- Average/one-shot latency is under budget; p95/p99 and concurrent verifier
  load are unknown.
- Counting node scale alone understated the prior workload. Three policies per
  node materially increased the 512-node input from about 796 KB to 2.58 MB
  and wall time from about 454 ms to 1.38 s, still below the 8 s budget.
- Repeating semantic-profile evolution three or ten times increases retained
  compiler, schema, corpus, and review-contract maintenance even when each
  individual profile addition is locally small.
- Repeating report retention remains source-bound; host-dependent timings and
  binary digests remain observations and therefore cannot independently prove
  reproducible fleet performance.

## Counterfactuals and migration valley

### A — retain the current implementation

Keep exact profile dispatch, full recompile, three historical manifest locks,
current-binary CI measurement, and proposal-only migration. This preserves the
strongest authority boundary. The steady-state costs are historical-profile
maintenance and full-verification runtime.

### B — minimal local improvement

Retain the three profile-0 full bundle/input byte corpora in addition to their
manifest addresses. This improves offline recovery and reviewer inspection
without changing compilation or deployment authority. It adds repository size
and fixture-governance cost but has a small rollback surface.

### C — cross-boundary lifecycle structure

Retain immutable full bundle/input corpora and, once several semantic profiles
exist, select exact verifier implementations through a content-addressed
registry. A replaceable cache may sit in front only when misses and sampled
hits re-run the canonical verifier. This improves long-term isolation but adds
artifact provenance, availability, sandboxing, registry retention, and
supply-chain ownership.

Migration valley:

- During extraction, shared and frozen profile implementations coexist and
  every retained corpus must pass both before the old path is removed.
- Registry adoption temporarily duplicates in-process and external verifier
  paths. Rollback keeps the in-process full recompile until registry evidence
  is complete.
- Tightening retained-corpus or performance evidence may invalidate checked-in
  evidence; rollback is to regenerate valid evidence, never to weaken the
  canonical bundle verifier.

## Total cost and scoring

`C_total` includes compiler implementation, schema/review integration,
coordination, runtime recompile, CI operation, future profile changes,
migration, authority risk, and opportunity loss. No production values were
available, so the following are ordinal scores only.

Scale: `E` externalized cost, `A` change amplification, `F` boundary failure,
`K` KPI divergence, `T` temporal lock-in; each 0–3. Severity is their sum.

| Candidate | E | A | F | K | T | Severity | Confidence | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|---|
| Shared historical lowering/corpus hashes | 1 | 2 | 1 | 1 | 3 | 8 | C2 | `time-delayed` |
| Full recompile | 1 | 1 | 1 | 1 | 1 | 5 | C2 | `harmless-locality` at measured scale |
| Manual contract inventory | 1 | 2 | 1 | 1 | 2 | 7 | C2 | compensated `time-delayed` |
| Proposal-only migration | 1 | 1 | 0 | 1 | 1 | 4 | C2 | intentional incomplete surface |

## Final verdict and next evidence

No remaining candidate reverses the authority design's benefit within the
audited two-profile experimental boundary. The previous critical findings are
closed: review/compiler identity is content-bound, historical defaulting cannot
authorize the current profile, migration identity substitution fails after
rehashing, verification counts the actual canonical recompile call, the
current benchmark exercises typed data edges and three policies per node, and
retained performance evidence refuses rehashed budget, workload, and source
substitutions.

Before stable promotion:

1. Retain the three profile-0 complete bundles/inputs as immutable bytes, or
   document why manifest content addresses plus retained compiler source are
   the chosen recovery corpus.
2. Add p95/p99 and concurrent verification evidence from representative
   production distributions, including larger edge and policy documents.
3. On the first semantic lowering change, create a new profile and prove all
   historical corpus outputs before extracting or modifying shared code.
4. Keep source-bound retained performance evidence current, while preserving
   the distinction between deterministic source/workload facts and observed
   binary/timing values.
5. Keep the five promotion blockers reported by promotion conformance open;
   this audit does not convert local compatibility evidence into stable or
   production evidence.

## Quality checklist

- [x] Explained local rationality before recommending changes.
- [x] Expanded `B`, `M`, `N`, and `T`.
- [x] Used structural, runtime, evolution, and semantic evidence.
- [x] Separated observed facts, inferences, and unobserved hypotheses.
- [x] Applied the candidate gate and traced compensation halos.
- [x] Compared function through lifecycle boundaries.
- [x] Tested metric and time-axis inversion.
- [x] Compared current, local, and cross-boundary counterfactuals, including the migration valley.
- [x] Scored `E/A/F/K/T` separately from confidence.
- [x] Avoided a stable/production claim from local bounded evidence.

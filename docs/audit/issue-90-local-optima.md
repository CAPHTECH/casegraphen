# Issue #90 implementation local-optima audit

## Scope and evidence

This audit covers the promotion-review refresh, its structured inventory, and
`promotion-review-conformance.py`. The inventory is an evidence index and
trigger ledger; it does not implement acceptance or stable-promotion authority.

## Evaluation frame

- **B — boundary:** retained evidence → structured fact → completed/open
  trigger → human promotion review.
- **M — metrics:** exact reference hash/date/commit, current surface counts,
  no already-satisfied open trigger, explicit non-promotion, and Issue owner.
- **N — needs:** stop repeated work on completed conditions, separate local
  evidence from external authority, and preserve a reviewable next trigger.
- **T — time:** current review, later evidence commits, surface/schema growth,
  and eventual stable proposal.

## Evidence planes

| Plane | Evidence | Remaining uncertainty |
| --- | --- | --- |
| Structure | Strict inventory keys, unique facts/triggers, issue ownership | Future status vocabulary |
| Execution | Conformance derives current workflow/runtime/contract counts | External evidence remains unavailable locally |
| Evolution | Exact hashes detect retained-evidence drift; completed/open sets are disjoint | Fact status updates remain reviewed edits |
| Meaning | Local/non-promotional/external statuses and explicit false decision | Final promotion always requires human authority |

## Ranked candidates

| Rank | Candidate | Local benefit | Externalized cost / inversion | Severity | Confidence | Verdict |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Keep a prose-only “next trigger” | Easy to write | Stale trigger causes repeated implementation and false blockers | High | High | Rejected; structured facts/triggers plus gate |
| 2 | Treat local passing evidence as provider authority | Fast promotion narrative | Collapses trust boundary and reviewer provenance | Critical | High | Rejected; separate non-promotional/external statuses |
| 3 | Copy canonical surface counts by hand | Readable snapshot | Drifts as workflows/contracts change | Medium | High | Rejected; conformance derives counts |
| 4 | Mix stable blockers with improvements | One convenient backlog | Optional design work can indefinitely block promotion | High | High | Rejected; separate required and optional sections |
| 5 | Mark mechanisms complete without retained release evidence | Rewards implementation | Inverts evidence and deployment authority | High | High | Rejected for #88 and #89 |

## Candidate detail and compensation halo

The former prose trigger was locally rational when few pilots existed. Its
compensation halo appeared later: four runtime families, a complete local
provider matrix, and an independent client all existed while the review still
asked for them. The inventory now names retained facts independently of the
prose and rejects an open trigger once all its facts are satisfied.

The same distinction prevents a second local optimum: declaring a repository
mechanism “done” because its unit tests pass. Issues #76, #88, and #89
retain stable blockers for evidence produced at the external or release
boundary, even where the implementation or bounded baseline already exists.
Issue #91 is separately satisfied by its governed exact-profile contract,
historical corpus, migration refusal tests, and source-bound performance gate;
that does not claim multi-release production history.

## Widened-boundary comparison

| Boundary | Prose-only review | Selected design |
| --- | --- | --- |
| One edit | Minimal | Inventory plus prose |
| Repository evolution | Silent drift | Exact hashes and derived counts fail CI |
| Maintainer workflow | Re-discovers completion manually | Completed/open trigger sets are explicit |
| Trust boundary | Local results can read as authority | Status distinguishes local, repository, and missing external evidence |
| Stable decision | Optional work can become accidental gate | Required blockers and post-v0 enhancements are separate |

## Counterfactuals

- **A — previous state:** human-maintained prose and stale trigger sentence.
- **B — narrow fix:** rewrite the paragraph with today's facts.
- **C — cross-boundary design (chosen):** exact evidence inventory,
  conformance-derived surface counts, satisfied-trigger detection, Issue
  ownership, and prose markers tied to structured IDs.

## Migration valley and rollback

The inventory initially depends on concurrent #87/#88/#91 documentation. Its
exact hashes must be finalized after those changes settle; conformance fails
closed on drift. Rollback can remove the inventory/gate and restore prose
without changing runtime or ledger data, but would reintroduce the stale-review
risk. No migration mutates accepted CaseGraphen state.

## False positives / non-candidates

- A missing external fact is not an implementation failure; it is deliberately
  distinct from a missing mechanism.
- Optional stable payload schemas, active/active persistence, IdP integration,
  and WORM mirroring are not hidden blockers.
- The inventory does not duplicate CaseGraphen decision rules; it governs only
  promotion-review evidence and delegates live counts to canonical artifacts.

## Residual evidence needed

The fact-status edit itself remains a reviewed human action. Future work could
sign the inventory or derive more facts from retained release records, but it
must not let a passing report promote the contract automatically. The next
observations are the external provider authority (#76), 10k/100k allocator
release reports (#88), and published runtime-durability/production-fleet
evidence (#89).

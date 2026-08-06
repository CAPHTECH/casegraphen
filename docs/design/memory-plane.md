# Memory Plane experimental v0

The Memory Plane turns accepted Case Graph structure into purpose-bound agent
context and turns external observations into **proposals only**. It adds no
authoritative store and no route around CaseGraphen review.

```text
source bytes -> Source Record -> unreviewed Memory Claim proposal
                                      |
                           existing review + morphism gates
                                      |
MorphismLog -> replayed CaseSpace -> Accepted Memory View
                                      |
                 exact/graph retrieval -> Memory Projection -> agent
                                      |
                         replaceable lexical/vector indexes
```

## Ownership boundaries

| Component | Owns | Must not own |
|---|---|---|
| Agent runtime | observation, action, optional use report | acceptance, authority elevation |
| Memory Plane | contract validation, derived status, retrieval, loss-explicit projection, proposals | durable truth, model execution, operation authority |
| CaseGraphen core | MorphismLog, replay, evidence trust rule, review, operation gates | autonomous learning, scheduling |
| Index adapter | lexical/semantic candidates after policy filtering | visibility, time, authority, acceptance |
| Human/policy | non-delegable review and authority | rewriting source history |

The Rust module accepts an already replayed `CaseSpace`. Query execution refuses
a `base_revision_id` different from that CaseSpace revision. This makes the
transaction-time cutoff observable without introducing a second persistence
API. A caller needing an earlier transaction time must replay that revision
first.

## Contracts

- `memory.source_record.v0`: immutable capture metadata and exact artifact hash.
- `memory.claim.v0`: typed reusable proposition, valid time, scope, source role,
  sensitivity, and authority ceiling. It deliberately has no acceptance field.
- `memory.query.v0`: actor/purpose/audience/scope/time/kind/budget request bound
  to an exact revision.
- `memory.policy.v0`: actor grants, valid-time requirements, and conflict rules.
- `memory.projection.v0`: selected structured claims plus sources, contested
  IDs, omissions, losses, authority summary, and content hash. It declares
  `read_only: true` and `accepted_state_changed: false`.
- `memory.use_report.v0`: caller report bound to the exact projection hash. It
  requires `self_reported: true` and `accepted: false`.
- `memory.index.v0`: deterministic lexical derivative with `derived: true` and
  `authoritative: false`.

All contracts live in `schemas/experimental`; incompatible changes remain
allowed until a separate stable-promotion decision.

## Claim materialization

A claim is represented by an evidence cell whose `metadata.memory_claim`
contains the strict typed contract. Every `source_ref` must identify a
`custom:artifact` cell whose `content_hash` matches its content-addressed ID,
and a `derives_from` relation must connect the claim to the artifact.
`metadata.memory_source_records` retains the strict Source Record contracts so
replay can recheck source-origin authority ceilings rather than trusting the
claim's provenance-role label. Every cited artifact must have a matching
Source Record hash.

Accepted status is derived, never written into the claim:

```text
candidate  = claim contract present but accepted review/trust/source incomplete
accepted   = central evidence trust rule + effective accepted review + sources
contested  = accepted claim with an accepted contradiction
superseded = accepted current superseder points to the claim
expired    = valid_until <= query as_of
retracted  = accepted current retraction points to the claim
rejected   = rejected lifecycle or effective review
```

`supersedes`, `retracts`, `contradicts`, and `authorized_by` affect the view
only when their relations are accepted. A superseder or retractor must itself
be an accepted, currently valid claim.

## Validation order

1. Strict contract/schema and exact revision.
2. Actor grant, audience, purpose, project, and sensitivity.
3. Claim scope and memory-kind route.
4. Effective accepted evidence status using the crate's one trust rule.
5. Immutable source reachability and authority ceiling.
6. Valid time and derived lifecycle.
7. Conflict classification.
8. Exact/graph candidates, then lexical relevance.
9. Item/token budget and loss recording.
10. Projection content address.

Ranking cannot cause an unauthorized, stale, unsupported, or hard-conflicted
claim to enter the candidate set. Semantic adapters must consume the filtered
set produced after step 7 and cannot add IDs.

## Proposal lifecycle

`build_claim_proposal` validates the source bytes, Source Record, authority
ceilings, and claim. Its result is an unreviewed proposed evidence cell and the
exact source artifact ID; it reports `accepted: false` and
`mutation_performed: false`. A host may submit that structure to the existing
CaseGraphen morphism workflow, but the Memory Plane never does so itself.

Revision, supersession, and retraction are new proposals and accepted relations,
not in-place writes. The original source and claim remain replayable.

## Index and raw-source escalation

The built-in v0 index is deterministic lexical material derived from a
projection. `index validate` rebuilds it from replay and compares the content
hash. Optional vector indexes follow the same contract and are not queried
until policy filters have run.

Retrieval tiers are:

1. accepted structured claims;
2. accepted graph neighborhood and linked evidence;
3. replaceable summaries/indexes;
4. exact immutable source fragments when the policy enables escalation.

The v0 projection returns exact source references so an agent or auditor can
perform tier 4 without treating the summarized claim as the source.

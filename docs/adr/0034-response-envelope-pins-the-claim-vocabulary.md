# ADR 0034: The Response Envelope Pins The Claim Vocabulary, Not Its Payloads

## Status

Proposed on 2026-08-07. Resolves the envelope decision of issue #120 and fixes
the two sibling records that issue requires; constrains the shape of issue #118
without designing it.

## Context

[`docs/product-surface.v0.json`](../product-surface.v0.json) is the Graph
Engineering v0 product-surface definition. It names
`casegraphen.experimental.control_plane.response.v0` as the wire response
schema for all 20 workflows and declares 14 invariants, among them
`proposals_are_never_auto_accepted` and
`memory_tools_never_mutate_accepted_state`. The named schema
([`control_plane.response.v0.schema.json`](../../schemas/experimental/control_plane.response.v0.schema.json))
constrains `result` as `{}` — the empty schema, which accepts anything. The
strings `accepted`, `mutation_performed`, and `read_only` appear zero times in
it. The product guide's end-to-end sequence terminates in "`accepted:false` +
unreviewed proposals"
([graph-engineering-product-surface.md](../guides/graph-engineering-product-surface.md)),
and that terminus — the product's actual deliverable — crosses the one boundary
where nothing contracts it. A hand-authored payload claiming `accepted: true`
validates against the shipped contract and is indistinguishable from a real
result to any consumer that validates. This repository's own
[`scripts/independent-mcp-client.py`](../../scripts/independent-mcp-client.py)
is exactly that consumer and, before this change, validated nothing against
this schema at all: it is deliberately Python-stdlib-only, so it made
hand-written field assertions (`assert_review_seam`) instead of importing
`jsonschema`, and no other layer checked its captured responses either. That
absence is part of why the gap went unnoticed — a validating consumer would
have had nothing to catch the forgery regardless, but an unvalidating one
could not have surfaced it even by accident. This change gives the client a
stdlib-only check (`forbidden_wire_claim`) over the same seven-key vocabulary
the schema now pins, proven against a real captured response it mutates in
place.

Issue #117 (resolved by the commit for #121) established the repair pattern for
a claim a tool can never truthfully make: pin it `const` **and** `required` in
the record's schema, because either alone is evadable — `const` by omitting the
field, `required` by setting it true
([`verification.policy_result.v0.schema.json`](../../schemas/experimental/verification.policy_result.v0.schema.json)).
That fix does not transfer mechanically here. `VerificationPolicyResult` had
one struct field hardcoded at one construction site; the control-plane claims
are raw `json!` literals at fourteen top-level tool-result construction sites
in [`src/bin/casegraphen-mcp-host.rs`](../../src/bin/casegraphen-mcp-host.rs)
(topology propose/lint, runtime attachment, bundle persistence, verification
lineage, resource reserve/release/reconcile, expansion, redesign, four memory
read shapes, the memory-proposal wrapper), plus a nested relation-proposal
record and a resource-read projection in the same binary, and three in
[`src/native_cli/ops/github.rs`](../../src/native_cli/ops/github.rs). There is
no single Rust field to pin, and the key set varies by tool: `accepted`,
`mutation_performed`, `read_only`, `accepted_runtime_output`,
`proofs_serialized`, `review_status`, `generated_plan_review_status` each
appear on the tools where they are that tool's claim.

Two facts, verified in this tree, anchor the decision:

1. **The envelope already pins invariants — just not this one.** The response
   schema pins `authority_facts.canonical_casegraphen_authorization` to the
   constant `not_evaluated`, and the sibling notification contract pins
   `authorizes_action` to `const: false`, with
   `ControlPlaneState::publish_notification` *forcing* that value in Rust at
   one protocol-layer point (`src/control_plane.rs:528`) before anything is
   journaled. The claim that a notification authorizes nothing is treated as a
   property of the wire; the claim that a result accepts nothing never got the
   same treatment.
2. **A blanket rule deeper than the top level would be wrong.** Read tools
   truthfully echo accepted ledger state *inside* their results: the `reviews`
   resource returns cells whose provenance is reviewed, and memory projections
   carry claim statuses. A rule of the form "no `accepted: true` anywhere under
   `result`" would make truthful reads schema-invalid. Only a payload's own
   contract knows which nested occurrence of a key is a proposal claim and
   which is an observation of ledger fact. The top level of `result` is
   different: it is authored by the host delegate, never echoed from data, and
   the host performs no acceptance — acceptance exists only behind the
   canonical review morphisms and operation gates, which this surface does not
   drive.

The three sibling types issue #120 found while auditing #117:

- `StageReleaseProposal.accepted`
  (`src/streaming_reconciliation.rs`, hardcoded `false` at its single
  construction site inside `reconcile_stream`) crosses the wire nested in the
  `reconcile_streaming_run` result, which has **no schema at all**.
- `MemoryClaimProposal.accepted` / `.mutation_performed`
  (`src/memory/model.rs`, hardcoded at the single `build_claim_proposal`
  site) crosses the wire through the four `memory_propose_*` MCP tools *and*
  the `casegraphen memory propose` CLI
  (`src/native_cli/ops/memory.rs`). `memory.claim.v0` covers the claim input,
  not this proposal wrapper.
- `DeclaredLineageReconciliation.ledger_requirements_satisfied`
  (`src/verification_policy.rs`, hardcoded `false`, asserted by
  `caller_declared_lineage_never_satisfies_ledger_requirements`) is library-only:
  no CLI or MCP path serializes it today, so there is no forgeable wire
  surface — but it is `pub` and `Serialize`, the exact posture
  `VerificationPolicyResult` was in before #117.

## Decision

Three layers, in decreasing generality. The envelope constrains what no payload
may ever claim; per-payload contracts constrain what each payload must say; a
single protocol-layer check makes the Rust side match the contract without
adding a second implementation of any rule.

The layers answer different threats, and only one of them is the forgery #120
opens with. Layer 1 is a schema, so it protects a **consumer validating what
it received** — including a hand-authored payload this host never emitted,
which is #120's attack. Layer 2 is a runtime check, so it protects against
**this host's own delegate** emitting a claim it must not, whether by defect
or compromise; no schema shipped to consumers can do that, because a buggy
host does not validate its own output against anything a consumer holds.
Layer 3 closes the evasions the first two structurally cannot see: omission of
a required claim, and claims nested below the envelope's altitude. Removing
any one layer on the belief that another covers it reopens that layer's
threat.

**1. `control_plane.response.v0` pins the claim vocabulary at the result's top
level.** `result` becomes `null` or an object. On the object, the seven
claim-bearing keys are constrained to their only truthful values *whenever
present*:

| key | pinned value |
|---|---|
| `accepted` | `const: false` |
| `mutation_performed` | `const: false` |
| `read_only` | `const: true` |
| `accepted_runtime_output` | `const: false` |
| `proofs_serialized` | `const: false` |
| `review_status` | `const: "unreviewed"` |
| `generated_plan_review_status` | `const: "unreviewed"` |

`result` stays otherwise open (`additionalProperties` unconstrained): the
envelope does not know its payloads, and freezing twenty experimental result
shapes into the transport contract is exactly the coupling ADR 0019's
protocol/delegate split exists to avoid. The envelope additionally pins the
response's success/refusal structure: exactly one of `result` and `refusal` is
non-null. A survey of every construction site confirms no current emission
violates any pin — the top level of every tool result carries these keys only
at their pinned values; the one raw report that carries `accepted` at its own
top level (`runtime.integration_report.v0`) already pins it `const: false` in
its own schema.

This makes a top-level acceptance claim **schema-inexpressible** for every
tool, including tools added later, without the envelope naming any tool. It is
the same treatment `authorizes_action` and
`canonical_casegraphen_authorization` already receive, extended to the last
invariant-bearing surface the envelope left open.

A consumer must read the guarantee at its exact altitude: **the envelope pins
the top-level claim, and a nested `accepted` is payload semantics, not an
envelope-level assertion.** `result.<anything>.accepted: true` at depth one or
below is unconstrained by this schema and indistinguishable, at the envelope,
from the truthful ledger echoes described in the context. A consumer that
needs a nested key governed must validate the payload against the payload's
own contract (layer 3). The schema's `description` states this boundary in
the same terms, because reading the pin as "no acceptance claim anywhere under
`result`" is exactly the misreading that would let a nested forgery ride an
envelope-validated response.

The `const`-without-`required` posture at this layer is deliberate, not an
evasion of #117's omission lesson: the envelope cannot require any claim key
because the response does not identify its tool, and requiredness is per-tool
knowledge (a `simulate_execution_topology` report legitimately carries none of
these keys). The omission half of the defense lives in layer 3, where each
payload contract knows what its record must say.

**2. One Rust enforcement point at the protocol layer.**
`ControlPlaneState::execute` — the single chokepoint every non-replayed tool
result flows through before journaling (`src/control_plane.rs`, immediately
after `delegate.invoke`) — checks the delegate's result against the same
vocabulary and converts a violation into an integrity refusal
(`noncanonical_wire_claim`) rather than journaling and emitting a false claim.
This sits at the same altitude as `publish_notification` forcing
`authorizes_action = false`, and it **deliberately diverges from that
precedent's disposition**. The notification is a record the protocol layer
constructs and owns outright, so forcing the value overrules no author and can
hide nothing. A tool result has an author — the delegate — and a delegate that
emits `accepted: true` is a defect or a compromise; rewriting its claim to
`false` would silently launder exactly the condition the check exists to
surface, and would let a compromised delegate keep operating behind a
protocol layer that cleans up after it. Refusing turns the event into
something the caller and the journal both see. Forcing is also mechanically
unavailable here: injecting or rewriting keys in results would break payloads
whose own strict contracts (`additionalProperties: false`) forbid deviation
from what their producer constructed. The `json!` literals remain what they
are — per-tool data — and enforcement has exactly one implementation, at the
layer that owns the wire.

**3. Per-payload contracts carry requiredness and the nested pins.** Three are
mandated now, all with #117's full pattern (`const` **and** `required`, schema
self-identification via a `schema` field set at the single construction site,
registration in
[`contracts.v0.json`](../../schemas/experimental/contracts.v0.json) so
`casegraphen schema get` serves them per ADR 0033, and the conformance gate
holding schema, Rust owner constant, and validating example together):

- `casegraphen.experimental.streaming.reconciliation.v0` — contracts the whole
  `reconcile_streaming_run` result (`StreamingReconciliation`), pinning
  `stage_release_proposals[*].accepted` as `const: false` + `required`. The
  proposal record is contracted through its parent because it is constructed
  nowhere else.
- `casegraphen.experimental.memory.claim_proposal.v0` — contracts
  `MemoryClaimProposal`, pinning `accepted` and `mutation_performed` as
  `const: false` + `required`. One contract covers both wire exposures: the
  MCP `memory_propose_*` results embed it as `claim_proposal`, and
  `casegraphen memory propose` emits it directly.
- `casegraphen.experimental.memory.relation_proposal.v0` — contracts
  `MemoryRelationProposal`, pinning `accepted` and `review_status` as
  `const: false` / `const: "unreviewed"` + `required`. This is the "nested
  relation-proposal record" the Context section's audit found and this
  Decision originally left unlisted: it crosses the wire nested inside the
  `memory_propose_supersession` / `memory_propose_retraction` results as
  `relation_proposal`, so — like the other two contracts above — it needed a
  layer-3 contract precisely because layers 1 and 2 are top-level-only by
  design and do not see it. The adversarial-execution-reviewer's pass on this
  ADR's implementation found the omission; it is fixed here, not deferred,
  because unlike `DeclaredLineageReconciliation` this record already crosses
  the wire today.

**Deliberately deferred:** `DeclaredLineageReconciliation` gets no contract in
this change. It has no wire surface to forge; contracting it now would mint a
schema for a record no consumer receives. The deferral is recorded here and as
a comment on the struct pointing at this ADR, so whoever plumbs it through a
CLI or the host inherits the obligation explicitly instead of rediscovering it.

**Deliberately out of scope: `resources/read` bypasses this envelope, and the
same claims do cross it.** MCP resource reads are wrapped as
`contents[].text` in `src/mcp_stdio.rs` and never pass through
`ControlPlaneResponse` or `ControlPlaneState::execute`, so neither layer 1 nor
layer 2 covers them — and the path is not claim-free: the `halts`/`runs`
projections carry a top-level `accepted: false` (`read_external_projection`
in the host binary), and the space reads echo ledger state. The boundary is
therefore **not uniform**: the pinned guarantee holds on `tools/call` and
does not exist on `resources/read`. This ADR does not absorb that surface
because there is no declared contract there to tighten —
`product-surface.v0.json` names a wire schema for the 20 tool workflows only,
and resource-read contents have no schema identity at all — so constraining
them means minting a new contract for an undeclared surface, a separate
decision with its own triage (which resources carry claims versus pure ledger
echoes). It must be filed as its own issue rather than assumed covered; until
then, a consumer must not read resource-read contents as governed by
`control_plane.response.v0`.

**What this implies for #118 (the input mirror).** The same architecture
answers the request side, with one asymmetry that changes the urgency. The
request envelope needs no equivalent pin: a request carries no claim for a
consumer to trust — the one trust-adjacent input,
`caller_declared_audit_context`, is already structurally quarantined in the
envelope and answered by the response's `authority_facts` constant, and every
trust value that matters is computed or forced by the tool. So #118 is a
publication problem, not a forgery problem: per-tool *input* contracts attach
to the tools (registered and served exactly as layer 3's contracts are), the
request envelope's `payload` stays open, and the loop closes with a
conformance gate that fails when a host input type has no registered contract.
Which of the sixteen input types are genuinely external versus host
configuration is #118's own triage and is not settled here.

## Consequences

- A payload claiming `accepted: true`, `mutation_performed: true`,
  `read_only: false`, or a reviewed status at the result's top level fails
  validation against the shipped `control_plane.response.v0` — for every
  current tool and every future one. A forged
  `stage_release_proposals[*].accepted: true`, forged/omitted
  `MemoryClaimProposal` claim, or forged/omitted `MemoryRelationProposal`
  claim fails the three new payload contracts. All three are proven by
  constructing the forgery against live host output and watching validation
  reject it, per the issue's acceptance criteria.
- What the envelope still does **not** guarantee: which tool produced a
  result, the result's shape, or the *presence* of any claim key. Those arrive
  payload-by-payload, on demand, as layer-3 contracts — the three mandated here
  plus whatever #118's triage and future consumer needs justify. The envelope
  guarantees only the product invariant: nothing crossing this wire can claim
  acceptance.
- `scripts/independent-mcp-client.py` and every existing response-validating
  test inherit the tightened contract with no code change; the live-emission
  sweep in the implementation plan proves every supported tool still
  validates.
- Old durable journals written by earlier hosts are not migrated; experimental
  v0 carries no backward-compatibility obligation, and a replayed response
  from this host version was checked when first computed.
- Stable promotion: the declared surface and the declared wire contract stop
  disagreeing about the central invariant, which repairs a latent
  misrepresentation rather than adding a blocker. The promotion ledger's
  current required blockers (provider authority, production fleet) are
  untouched and this change changes no promotion decision — it does touch
  `docs/reviews/graph-engineering-v0-promotion.inventory.json`'s
  `experimental_contract_count`, a mechanical count of governed schemas that
  two new contracts (streaming/memory claim-proposal) necessarily moves, but
  `decision`, `required_stable_blockers`, and `completed_local_triggers` are
  untouched. One recommendation is recorded: #118's external-input contracts
  should be settled before this surface is promoted, because integrators
  construct inputs against whatever is published, and today that is Rust
  source.

## Rejected alternatives

- **`result` stays `{}` and the invariants live only in per-tool contracts.**
  Nothing binds `result` to any of those contracts, so each is advisory at the
  boundary — #120's own finding. The product's terminus would remain
  uncontracted exactly where it is delivered.
- **A discriminated union of per-tool result schemas.** The response carries no
  tool discriminator, so the union cannot dispatch; adding a `tool` field is
  envelope surgery that then makes every new tool and every result-shape
  change an envelope contract event, coupling the transport contract to
  delegate decisions across twenty workflows. Its forgery resistance is also
  only as strong as the *most open* branch: one branch without
  `additionalProperties: false` readmits the forged key, whereas the vocabulary
  pin applies regardless of branch. And it would freeze seventeen ad-hoc
  experimental shapes prematurely to deliver an invariant the pin delivers
  alone.
- **Requiring a `schema` self-identifier in every result now.** The natural
  companion to layer 3, and the likely end state — but mandating it today
  forces minting contracts for all remaining results at once for zero
  additional invariant strength. It becomes the obvious follow-on whenever a
  consumer needs a result's shape, not its claims.
- **Typed result structs replacing the `json!` literals.** Twenty
  heterogeneous shapes; typing them all is a host rewrite that still needs the
  schema to bind consumers, and per-struct hardcoding would place the rule in
  twenty places — the two-implementations failure `CLAUDE.md` forbids. The
  single `execute` check gives the Rust-side guarantee at one point.
- **Pinning the vocabulary recursively at every depth.** Forbids truthful
  reads: reviewed cells and memory claim statuses legitimately surface
  accepted ledger state nested inside read results.

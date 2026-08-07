# ADR 0036: The Claim Vocabulary Pin Extends To Resource Reads, By Self-Identification Rather Than Envelope Surgery

## Status

Accepted on 2026-08-07. Resolves issue #122, the surface ADR 0034 named
"deliberately out of scope" and required be filed as its own issue rather
than assumed covered. Supersedes that section of ADR 0034; see the amendment
recorded there.

## Context

ADR 0034 pinned the seven-key claim vocabulary (`accepted`,
`mutation_performed`, `read_only`, `accepted_runtime_output`,
`proofs_serialized`, `review_status`, `generated_plan_review_status`) at the
top level of `control_plane.response.v0`'s `result`, and added a matching
runtime check in `ControlPlaneState::execute`. Both cover `tools/call` only.
`resources/read` bypasses both: `src/mcp_stdio.rs` wraps resource content as
`contents[].text` and it never passes through `ControlPlaneResponse` or
`execute`. ADR 0034 recorded that the same vocabulary crosses that boundary
too — the `halts`/`runs` projections carry a top-level `accepted: false` —
and left it unaddressed because there was no declared contract there to
tighten and no chokepoint identified, deferring the triage to this issue.

**Classification of the seven `RESOURCE_TEMPLATES`** (`src/control_plane.rs`),
read by inspecting every handler arm of `OperationalDelegate::read_resource`
(`src/bin/casegraphen-mcp-host.rs`) and every struct or function each arm
returns:

| resource | shape | classification |
|---|---|---|
| `spaces/{id}/status` | `{case_space_id, current_revision_id, evaluation}` | pure echo |
| `spaces/{id}/frontier` | `{case_space_id, current_revision_id, readiness}` | pure echo |
| `spaces/{id}/reviews` | `{case_space_id, current_revision_id, review_gaps, reviewed_cells}` | pure echo |
| `spaces/{id}/revisions/{revision}` | `NativeRevisionRecord` (`revision_id, parent_revision_id, sequence, entry_id, morphism_id, snapshot_path, source_ids, replay_checksum`) | pure echo |
| `spaces/{id}/halts` | `read_external_projection` output | claim-bearing |
| `runs/{run_id}` | `read_external_projection` output | claim-bearing |
| `topologies/{topology_id}` | `read_external_projection` output | claim-bearing |

The four pure-echo resources never place any of the seven vocabulary keys at
their own top level. `review_status` appears only nested, inside
`reviews`'s `reviewed_cells[*].provenance` — a real, ledger-observed review
status, exactly the truthful-echo case ADR 0034's second anchor fact
describes and protects by pinning the top level only. The three claim-bearing
resources share one construction site, `read_external_projection`
(`src/bin/casegraphen-mcp-host.rs`), which already hardcoded a top-level
`accepted: false` at that one site — the same posture `MemoryClaimProposal`
and `StageReleaseProposal` were in before #117, and simpler than the
`tools/call` case ADR 0034 addressed: fourteen heterogeneous `json!`
construction sites there, one shared function producing one shape here.

**Is `resources/read` in the declared product surface?** No.
`docs/product-surface.v0.json` named `wire_response_schema` for the 20
`tools/call` workflows only. No skill, no doc, and
`scripts/independent-mcp-client.py` never reference a `casegraphen://` URI
or exercise `resources/read`. Yet the host advertises the surface live:
`initialize` returns `capabilities.resources`, and
`resources/templates/list` serves all seven templates unconditionally. A
live, reachable surface with no declared identity is the more surprising of
the two branches the issue named.

**Is there a chokepoint?** Yes.
`control_plane::read_resource` is the single function every resource read
flows through before reaching the wire — called from exactly one place,
`mcp_stdio.rs::read_resource_request` — the same shape as
`ControlPlaneState::execute` for `tools/call`. Because no pure-echo resource
ever surfaces the vocabulary at its own top level (established above by
inspection, not assumed), a top-level-only pin at this chokepoint does not
invalidate any current truthful read.

## Decision

**1. Declare the surface.** `docs/product-surface.v0.json` gains a
`resources` section naming all seven templates, each tagged
`claim_bearing` or `pure_echo`, with the claim-bearing three naming their
contract. The pure-echo four are declared open by design — the file says so
— rather than left silent, which is the distinction that matters: "declared
open" and "never mentioned" must not look the same to a reader.

**2. Layer 2: a top-level-only vocabulary pin at `read_resource`.**
`control_plane::read_resource` now checks a successful delegate result
against the same seven-key vocabulary, at the top level only, and refuses the
read (`noncanonical_resource_wire_claim`) rather than letting a forged or
defective top-level claim reach the wire. The vocabulary and the truthful-value
comparison are extracted into a shared predicate, `claim_vocabulary_violation`,
called from both `wire_claim_violation` (`tools/call`) and `read_resource`
(`resources/read`) — the two paths ask the exact same question at the same
altitude, so the rule has exactly one implementation, per `CLAUDE.md`. What is
*not* shared is the refusal construction: `tools/call` converts a violation
into an envelope refusal (`noncanonical_wire_claim`, journaled into
`ControlPlaneResponse.refusal`), while `resources/read` has no envelope to
journal into and refuses the read directly
(`noncanonical_resource_wire_claim`). Sharing that part too would force one
of the two call sites to fake the other's envelope shape, which is exactly
the kind of coupling that makes both harder to read — so it is kept separate,
deliberately, not by oversight.

One asymmetry from `execute` is deliberate, not an inconsistency: `execute`
also refuses any non-object top-level result, because `tools/call`'s
`result`/`refusal` exclusivity `oneOf` makes a non-object result itself
malformed. `resources/read` has no such envelope, so a non-object resource
value is not a violation of anything — it simply has no top-level key for the
vocabulary to apply to, and the check is a no-op, not a refusal.

**3. Layer 3: contract the claim-bearing shape.**
`casegraphen.experimental.control_plane.resource_projection.v0` governs the
`read_external_projection` output — the single shape `halts`, `runs`, and
`topologies` all return. `accepted` is pinned `const: false` **and**
`required`, #117's pattern (either alone is evadable: `const` by omitting the
field, `required` by setting it true). Registered in
`schemas/experimental/contracts.v0.json` so `casegraphen schema get` serves
it, with a validating example.

**4. Layer 1, achieved by self-identification, not envelope surgery.**
`resources/read` wraps content as an opaque JSON string in `contents[].text`;
there is no envelope field to carry a schema id, and no place for a consumer
to hook a validator without restructuring how MCP resource contents are
wrapped — a redesign of the transport, not a fix to this gap. That
restructuring is not done here. Instead, `ResourceProjection` — the Rust type
`read_external_projection` now serializes instead of a bare `json!` literal —
gained a `schema` field, exactly as `VerificationPolicyResult` gained one in #121
for the same reason. A consumer that parses `contents[].text` sees which
contract governs what it just read and can validate against it directly, all
without the envelope knowing anything about resource payloads. That is layer
1's value for anyone who validates, reached without opening the "how does a
resource content id itself to the envelope" question at all.

**Deferred, and the deferral is scoped precisely:** *where* a resource
content's schema is announced — inside an MCP-level structured-content field
the protocol itself understands, versus inside the content as done here — is
left open, the way ADR 0034 deferred `DeclaredLineageReconciliation`. This is
not a deferral of *whether* the shape is contracted: after (3), it is. If a
future consumer needs the schema identity before parsing `contents[].text`
(rather than after), that is the question left for whoever needs it next.

## Consequences

- A forged top-level `accepted: true` (or any of the other six keys' forbidden
  value) on any resource read is refused at `read_resource`, proven by a
  fixed `ResourceDelegate` test double returning exactly that value and
  observing `noncanonical_resource_wire_claim` — the same style ADR 0034 used
  to prove `wire_claim_violation` for `tools/call`.
- A forged `accepted: true` mutation of a live, spawned host's real
  `resource_projection.v0` response fails `python3 -m jsonschema` validation
  against the shipped schema, proven against real host output, not asserted
  in the abstract.
- The four pure-echo resources (`status`, `frontier`, `reviews`,
  `revisions/{revision}`) read unchanged, proven by a live spawned-host test
  reading all seven templates against a real store and real artifact files —
  the regression the top-level-only choice exists to avoid, checked rather
  than assumed.
- `docs/reviews/graph-engineering-v0-promotion.inventory.json`'s
  `experimental_contract_count` moves from 70 to 71, mechanically, from the
  one new contract; `decision` and `required_stable_blockers` are untouched,
  and this change touches no promotion decision.
- The `tools/call` guarantee is unchanged: `wire_claim_violation`,
  `execute`'s enforcement, and `control_plane.response.v0` are not modified
  in behavior, only refactored to share the vocabulary predicate.

## Rejected alternatives

- **Recursive pin on resource content.** Forbids truthful reads exactly as
  ADR 0034 already found for `tools/call`: `reviews`'s `reviewed_cells`
  legitimately carry a real `review_status` nested below the top level.
- **A structured-content envelope for `resources/read` now**, giving the
  protocol layer its own schema-id field the way `tools/call`'s response
  does. Rejected for this change because it restructures the MCP wire wrapper
  for a gain layer 1 already gets more cheaply via self-identification, and
  because ADR 0034's own precedent is to defer a structural question with no
  live urgency (`DeclaredLineageReconciliation`) rather than build it
  speculatively.
- **Leaving `resources/read` undeclared and uncontracted**, on the theory
  that no shipped skill exercises it today. Rejected: the surface is live and
  advertised regardless of whether a skill uses it, and an undeclared path
  carrying claim-shaped data is exactly the surprising state the issue exists
  to close.

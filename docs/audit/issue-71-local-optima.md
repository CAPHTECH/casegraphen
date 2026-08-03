# Issue 71 implementation local-optima audit

## 1. Executive summary

- Mode: `intervention` after authorization-boundary tests passed.
- Scope: MCP request/response schemas, `control_plane`, stdio transport,
  operational host behavior, product/Skill conformance, ADR and guides.
- System outcome: host access is authenticated without turning caller-supplied
  attribution into CaseGraphen capability authorization.
- Result: the ambiguous MCP `operation_gate` was replaced by a caller-declared
  audit context, and bearer authentication and canonical CaseGraphen authority
  are recorded as separate facts. The audit found and fixed one migration
  hazard: old durable response records now load conservatively without
  inventing authority.
- Evidence constraint: the host remains stdio and acceptance-ledger mutations
  remain refused, so no production network identity/SLO evidence exists.

## 2. Evaluation conditions

| Variable | Initial boundary | Expanded boundary |
|---|---|---|
| `B` | one request presence check | client -> authenticated MCP transport -> durable replay -> delegate -> CLI/store mutation owner |
| `M` | required fields present | non-confusable authority, denial behavior, audit provenance, restart migration |
| `N` | rename one Rust field | schemas, types, hashes, transport, docs, conformance, migration read path |
| `T` | new request | old journal restart and future acceptance-ledger delegation |

## 3. Evidence

| Plane | Evidence | Constraint |
|---|---|---|
| Structure | `src/control_plane.rs`, `src/mcp_stdio.rs`, request/response schemas, ADR 0021 | static ownership only |
| Execution | control-plane, MCP, product-surface, and Skill conformance tests | local stdio host |
| Evolution | old response deserialization counterexample; experimental v0 change | no production journal sample |
| Meaning | all audit fields are `declared_*`; authority facts say `not_evaluated` | custom embedders still own their authentication truth |

## 4. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Caller input named `operation_gate` | reused familiar shape | clients infer capability authorization that never occurred | trust boundary | 12 | C3 | mixed; fixed |
| 2 | New response fact required in durable Rust decode | strict new output | old journal cannot restart | lifecycle | 8 | C2 | time-delayed; fixed |
| 3 | Transport auth fact outside canonical response | preserves transport-neutral core | consumer must inspect both envelope and structured response | adapter boundary | 5 | C2 | harmless-locality |

## 5. Candidate details

### C1 — Audit input masquerading as authorization

**Facts.** The prior control plane required only `operation_gate.is_some()` and
never called `check_operation_gate`. The operational host token was the actual
access control, while acceptance mutations were refused. Tests now show a full
caller context without a token is rejected before delegation, and a valid token
plus declared capabilities records canonical authorization as `not_evaluated`.

**Local rationality.** Reusing the native gate vocabulary minimized types and
made clients provide useful attribution. The adapter author benefited; the
security meaning was paid by every client/reviewer.

| Boundary | Prior design | Implemented alternative | Advantage |
|---|---|---|---|
| function | presence check is smallest | renamed typed context | prior |
| protocol | familiar but misleading | explicit declared fields and authority facts | alternative |
| product | authority is ambiguous | token and ledger gate have distinct owners | alternative |
| lifecycle | future code may trust the field | conformance rejects reintroduction | alternative |

Counterfactuals: A) preserve the name and document it (low migration, ambiguity
remains); B) validate every tool as a CaseGraphen capability operation (requires
invented operations and authority); C) bearer-authorized host plus audit-only
context and canonical CLI/store gates for ledger mutation (chosen). `E=3,
A=2, F=2, K=3, T=2`, severity 12, confidence C3, classification `mixed`.

### C2 — Strict response evolution blocks old journals

**Facts.** `ControlPlaneResponse` is stored in the durable replay journal. A new
non-defaulted `authority_facts` field would make serde reject every old response
record before the host starts. The output JSON Schema should require the field,
but the persisted-input reader now uses a conservative default: caller context
absent and canonical authorization `not_evaluated`. A regression test parses an
old response shape and proves no authority is synthesized.

**Local rationality.** Strict serde catches malformed state and keeps output and
Rust types identical. Operators restoring a journal bear the migration outage.

| Boundary | Strict-only | Conservative migration read | Advantage |
|---|---|---|---|
| new response | exact | exact | tie |
| restart | old state refuses | old state loads as no authority | alternative |
| audit | missing provenance | explicitly unknown/not evaluated | alternative |

Counterfactuals: A) break old journals; B) migration command; C) default only
the new fact on read while always emitting it on write (chosen). `E=2, A=1,
F=2, K=1, T=2`, severity 8, confidence C2, classification `time-delayed`.

### C3 — Transport facts outside the control-plane response

The token is verified in `McpStdioServer`, while `ControlPlaneState` is
transport-neutral. Keeping bearer facts in the MCP envelope and canonical
authorization facts in the structured response creates two inspection points,
but avoids letting a caller-constructible transport-neutral request claim it
was authenticated. Both are mandatory in tool results and tests assert both.
No advantage inversion was found within the investigated boundary; verdict
`harmless-locality`, severity 5, confidence C2.

## 6. Compensation halo and false-positive guards

The removed compensation was prose explaining why an apparent gate was not a
gate. Schema names, field prefixes, response facts, tool discovery, refusals,
ADR, product inventory, and Skill checks now carry the same distinction.
Native CLI/store operation gates were intentionally not renamed: they really do
call canonical authorization and are a bounded-context distinction, not drift.

## 7. Remaining evidence

| Priority | Evidence | Uncertainty | Method |
|---:|---|---|---|
| 1 | restart with a real pre-#71 journal | complete state migration beyond response shape | archived fixture replay |
| 2 | future ledger mutation delegate test | canonical CLI/store owner remains mandatory | authenticated E2E denial/authorization matrix |
| 3 | embedding-server auth audit | custom adapter records transport facts correctly | package-specific threat test |

Quality checklist: local benefits and burden owners recorded; facts/inference
separated; structural plus execution evidence used; boundary inversion and
A/B/C counterfactuals included; severity and confidence separated; migration
cost and intentional bounded-context differences considered.

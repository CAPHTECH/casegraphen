# Issue 73 implementation local-optima audit

## 1. Executive summary

- Scope: resource-bearing `reconcile_run`, its versioned expectation bundle,
  allocator provenance checks, and canonical resource reconciliation.
- Conclusion: runtime self-authorization and MCP-local decision duplication
  were avoided. Repeated join identifiers in the bundle are an intentional
  boundary contract, not a harmful local optimum on current evidence.
- High-confidence material candidates remaining: zero.
- Evidence constraint: local E2E tests exercise one resource-bearing run; no
  production payload-size, adapter-change, or operator-error history exists.

## 2. System outcome and B/M/N/T

System outcome: a resource-bearing runtime run can reach an independent review
seam only when its exact topology/revision, allocator-issued grant, runtime
allocation, artifact bytes, and canonical reconciliation all agree.

| Variable | Current condition | Expanded condition used by this audit |
|---|---|---|
| `B` boundary | MCP `reconcile_run` payload | runtime adapter, allocator journal, integration/resource protocols, review and resource-free clients |
| `M` metric | parse/validate one request and produce a report | authority preservation, substitution detection, compatibility, change amplification and operator usability |
| `N` change scope | host adapter and bundle type | schemas, runtime adapters, allocator, canonical reconciler, Skills and product surface |
| `T` horizon | one experimental-v0 run | repeated integrations, contract evolution and stable-product promotion |

Constraints: runtime output remains untrusted, proposals never auto-accept,
client-observed revisions are not rebased, and the host cannot own a second
resource decision rule.

## 3. Evidence

| Observation plane | Source | What it establishes | Constraint |
|---|---|---|---|
| Structure | `src/runtime_integration.rs`, MCP host | bundle validation owns identity/provenance joins; allocation comparison delegates canonically | static inspection |
| Execution | `tests/resource_expectation_bundle.rs`, `tests/resource_host_e2e.rs` | stale/substituted/duplicate inputs refuse; restart path reaches only `needs_review` | one-node local fixture |
| Evolution | strict v0 schema/inventory/Skill/product surface | contract changes are governed across consumers | no multi-version history |
| Meaning/organization | allocator exact-record checks and `accepted: false` | runtime reporter cannot mint its own grant or acceptance | real adapter/operator behavior unobserved |

## 4. Ranked candidates

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Repeated node/attempt/reservation/allocation joins in the bundle | detects substitution at one strict boundary | adapter authors repeat identifiers and update a governed schema | no inversion observed; possible under frequent contract churn | 3/15 | C2 | `not-local-optimum` currently |
| 2 | Optional resource bundle path | preserves resource-free compatibility | two host branches must remain semantically aligned | integration evolution | 2/15 | C2 | `harmless-locality` |

Historical alternatives—trusting compact runtime allocations as grants and
reimplementing comparison in the MCP host—are excluded because the implemented
boundary explicitly prevents them.

Secondary candidate score: the optional resource-free branch is
`E=0, A=1, F=0, K=0, T=1`, `Severity=2/15`, `Confidence=C2`. It is classified
`harmless-locality` because it preserves compatibility while both paths retain
the same canonical reconciler.

## 5. Detailed candidate card: repeated bundle joins

### Identification and fact/inference/hypothesis

- Target/owner: `ResourceExpectationBundle`, runtime integration boundary.
- [Evidence] node, attempt, declaration, reservation, allocation and
  disposition identifiers repeat across nested records and must be equal.
- [Evidence] unit tests show stale, substituted and duplicate records refuse.
- [Evidence] the host verifies exact allocator records before calling
  `reconcile_with_resources`; it does not compare grants independently.
- [Inference] adapter authors perform additional serialization and must update
  more fields when identities change.
- [Hypothesis] frequent contract evolution or very large bundles could make
  that repetition cost exceed its substitution-detection value.

### Local rationality and B/M/N/T

- Local purpose/metric: make all authority and runtime joins explicit and
  fail-closed within one versioned request.
- Beneficiaries: reviewers, runtime integrators, and the acceptance boundary.
- Current benefit: resource-bearing runs can complete reconciliation without
  allowing runtime output to claim its own grant.
- Still-valid constraints: exact revision/hash binding, canonical allocator,
  untrusted runtime, experimental v0.
- `B`: one reconciliation request; `M`: correctness/provenance; `N`: bundle and
  host; `T`: one run. Expansion to many adapter versions creates the hypothesis.

### Compensation halo

| Local decision | Boundary effect | Compensation | Cost bearer | Frequency/scale | Evidence |
|---|---|---|---|---|---|
| explicit repeated joins | larger/more verbose adapter payload | typed schema, examples and conformance tests | runtime adapter maintainer | every resource-bearing report | schema and tests; size not measured |
| optional resource path | two reconciliation branches | shared `GenericJsonlReconciler`; resource branch only derives expectations | host maintainer | every reconcile change | host structure and E2E tests |

### Four observation planes

- Structure: repeated fields enforce equality but do not introduce a second
  grant representation; allocator records remain canonical.
- Execution: negative tests and restart E2E pass; throughput/size is unmeasured.
- Evolution: inventory and cross-contract tests expose coordinated changes.
- Meaning/organization: runtime adapter authors pay verbosity; reviewers and
  operators receive explicit provenance and fail-closed diagnostics.

### Boundary expansion and inversion

| Boundary | Current benefit | Current cost | Less-repetitive alternative benefit | Alternative cost | Advantage |
|---|---|---|---|---|---|
| Function | simple equality checks | repeated fields | fewer comparisons | implicit lookup context | current |
| Module | strict typed joins | serializer verbosity | smaller bundle type | hidden dependency on external stores | current |
| Feature | substitution detection | adapter assembly | compact resource reference | extra resolution/failure modes | current |
| System | provenance crosses process boundary | payload size | centralized lookup | online coupling and availability dependency | current on evidence |
| Operations | self-describing failure input | more bytes to inspect | shorter request | separate state gathering | unresolved |
| Lifecycle | governed explicit evolution | cross-contract updates | fewer fields to migrate | weaker offline replay/audit | unresolved; no inversion observed |

- Minimum inversion boundary: none demonstrated.
- Potential inverting metric: adapter change amplification or payload cost.
- Potential horizon: repeated stable-contract revisions; currently hypothetical.

### A/B/C counterfactual and migration valley

#### A. Maintain the versioned explicit bundle

- Steady state: self-describing request, strict substitutions checks, governed
  adapter work.
- Future cost/risk: schema migrations touch runtime adapters and examples.
- Rollback need: none.

#### B. Minimal local improvement

- Change: add builder/helper APIs and payload-size/change-amplification
  telemetry while leaving the wire contract unchanged.
- Benefit: reduces adapter mistakes and establishes whether repetition is costly.
- Remaining problem: wire repetition remains.
- Migration valley: old and new adapter construction coexist; documentation and
  SDK support temporarily expand.
- Rollback: callers can return to direct typed construction.

#### C. Cross-boundary structural change

- Change: replace embedded allocator records with a content-addressed,
  versioned resource-expectation artifact resolved by the host, while retaining
  exact topology/revision binding and canonical reconciliation.
- Preconditions/owners: allocator, artifact store, MCP host, runtime adapters,
  schemas and operations agree on availability and content addressing.
- Steady benefit: smaller repeated requests and reusable expectation artifacts.
- New cost/coupling: online artifact resolution, lifecycle/retention authority,
  and another availability boundary.
- Migration valley: dual support for embedded and referenced bundles, parity
  checks, more failure modes, and temporarily larger test/operational surface.
- Rollback: retain embedded v0 support until referenced artifacts have complete
  parity; then disable reference resolution if needed.

### Score and verdict

- `E` externalization: 1 (adapter verbosity/change work).
- `A` change amplification: 1 (governed schema plus consumers).
- `F` boundary failure: 0 (strict failures are intended; no incident observed).
- `K` KPI divergence: 0 (local validation supports the system authority goal).
- `T` time lock-in: 1 (v0 and typed helpers keep migration feasible).
- `Severity`: **3/15**.
- `Confidence`: **C2** that repetition and substitution protection exist;
  inversion is unverified.
- Classification: `not-local-optimum` on current evidence. Reassess if adapter
  churn or payload measurements demonstrate an advantage reversal.

## 6. Cross-cutting compensation structure

Schema inventory, examples, Skill conformance and E2E fixtures are the main
compensation for a strict cross-boundary contract. They are borne by repository
maintainers on contract changes, but currently prevent drift rather than hide
an inconsistent authority model.

## 7. Rejected false positives

| Target | Initial signal | Rejection reason | Rationality |
|---|---|---|---|
| resource-free compatibility branch | two code paths | both use one reconciler and only resource-bearing input needs allocator provenance | bounded backward compatibility |
| exact allocator lookup | extra I/O before reconciliation | prevents a runtime from constructing its own grant | authority boundary |
| `accepted: false` after complete reconciliation | apparently incomplete automation | acceptance and runtime observation deliberately have different owners | review seam, not inefficiency |

## 8. Unverified items and next evidence

| Priority | Evidence | Uncertainty resolved | Method |
|---:|---|---|---|
| 1 | bundle byte size and reconciliation p95 by node count | whether repetition causes system cost | pilot with 10²–10⁴ resource nodes |
| 2 | adapter PR/change history across contract revisions | actual change amplification | Git co-change analysis after multiple revisions |
| 3 | runtime/operator error taxonomy | whether explicit joins reduce or increase integration failures | pilot logs and support records |

No new material finding emerged; implementation changes are not justified by
the evidence currently available.

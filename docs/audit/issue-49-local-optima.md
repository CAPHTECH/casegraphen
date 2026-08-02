# Issue #49 implementation local-optima audit

## 1. Executive summary

- Scope: `src/graph_compiler.rs`, canonical topology serialization, and the
  generic JSONL compiler fixture.
- System outcome: lower an exact reviewed or proposal topology into inspectable,
  byte-stable artifacts without granting review or deployment authority.
- Evidence: source structure, Issue #49 invariants, ADR 0002, compiler unit
  tests, and the existing topology/linter/plan validators. No production runtime
  traces or organizational metrics exist for this new path.
- Result: one harmful local optimum was found and fixed. No high-confidence
  major candidate remains after the fix.

## 2. Evaluation conditions

| Variable | Local condition | Expanded condition |
|---|---|---|
| `B` | one topology-to-JSON function | case review, plan acceptance, runtime adapter, audit consumer |
| `M` | successful artifact generation | authority preservation, reproducibility, migration and operator cost |
| `N` | compiler module and experimental artifacts | topology/linter/review APIs, without changing stable plan semantics |
| `T` | first generic JSONL target | repeated target additions and eventual stable-contract promotion |

The local purpose is deterministic lowering. The wider purpose is preserving
CaseGraphen's acceptance boundary while making runtime deployment inspectable.

## 3. Evidence used

| Observation surface | Source | Scope | Constraint |
|---|---|---|---|
| Structural | `src/graph_compiler.rs`, `src/execution_topology.rs`, `src/graph_lint.rs` | canonicalization, validation reuse, bundle joins | static evidence |
| Runtime | five compiler unit tests | reorder stability, hash invalidation, fail-closed loss, lint refusal | synthetic only |
| Evolution | Issue #43/#45 APIs and ADR 0002 | boundary and change amplification | new code; no co-change history |
| Meaning/organization | `CLAUDE.md`, Issue #49 | review authority and decision-rule ownership | no operator interviews |

## 4. Ranked candidates

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | policy presence without identity validation (pre-fix) | generic documents require no policy parser | malformed policy could be labelled preserved | compiler/runtime safety seam | 10/15 | C2 | `externalization`, fixed |
| 2 | explicit per-node plan mapping | compiler avoids inventing authorization | caller must provide repetitive mapping | repeated large topologies | 5/15 | C1 | `harmless-locality` for v0 |
| 3 | manifest hash held outside manifest bytes | avoids recursive self-hash | consumers must retain bundle envelope | artifact transport | 3/15 | C1 | `not-local-optimum` |

## 5. Candidate 1: policy presence without identity validation

### Facts, inference, and hypothesis

- **Observed:** the initial implementation compared declared policy IDs with map
  keys, but accepted any JSON value as the corresponding document.
- **Observed:** runtime deployment says requirements are preserved and emits the
  policy files by content hash.
- **Inference:** a `null` document or a document naming a different `policy_id`
  could pass compilation while downstream consumers reasonably read the bundle
  as preserving the referenced safety/acceptance policy.
- **Hypothesis:** target-specific policy schema validation will be needed after
  real policy contracts exist; v0 has no evidence for those schemas yet.

### Local rationality

Opaque JSON keeps the compiler runtime-neutral and avoids duplicating future
verification, budget, and expansion decision rules. That remains a valid
benefit. Merely checking map presence, however, moved identity integrity to the
adapter/operator while the compiler claimed preservation.

### Compensation halo

`opaque document -> policy identity can disagree -> adapter must re-check or
operator manually inspects -> adapter maintainer/operator -> every compile`.

### Boundary inversion

| Boundary | Original benefit | Original cost | Adopted alternative | Alternative cost | Advantage |
|---|---|---|---|---|---|
| function | minimal generic check | none locally | object + exact `policy_id` check | small branch | adopted |
| module | no policy schema coupling | misleading preservation possible | identity integrity only | policy semantics stay opaque | adopted |
| runtime seam | adapter flexibility | safety requirement can disappear | exact referenced document shipped | adapter still interprets semantics | adopted |
| lifecycle | easy new policy shapes | repeated downstream checks | stable identity rule, evolvable bodies | future schemas still needed | adopted |

The earliest reversal is the runtime integration boundary. Score:
`E=3, A=1, F=2, K=3, T=1`, severity `10/15`, confidence `C2`, classification
`externalization`.

### Counterfactuals and intervention

- **A — original:** presence-only check; lowest implementation cost, unsafe
  preservation claim.
- **B — adopted:** require each document to be an object with an exact
  `policy_id`; preserves identity without implementing policy semantics.
- **C — full schemas:** validate every policy body. This could become superior
  once those contracts exist, but today would invent and duplicate decision
  rules. Migration would require versioned policy schemas and adapter changes.

The implementation adopts B and carries it with
`malformed_policy_document_cannot_be_claimed_as_preserved`.

## 6. Candidate 2: explicit plan mapping

The topology intentionally lacks stable `execution.plan.v1` worker bindings,
evidence-requirement IDs, and allowed transition classes. Inventing those values
would make compilation locally convenient but globally weaken preauthorization.
The compiler instead requires one explicit mapping per node and refuses missing
or incomplete mappings as acceptance-affecting information loss. The burden is
on the graph deployer at compile time; at v0 this is intentional duplication of
deployment facts, not a duplicated decision rule.

Score: `E=1, A=2, F=0, K=1, T=1`, severity `5/15`, confidence `C1`, verdict
`harmless-locality`. Reassess after several large real mappings show measured
change amplification.

## 7. Designs considered but not classified as local optima

| Design | Initial signal | Reason rejected | Rationality |
|---|---|---|---|
| opaque reviewed-mode binding | might trust caller review | binding fields are private and only the canonical CaseGraphen review log constructor can create it outside module tests | preserves single review decision rule |
| two compilation modes | two paths may drift | both modes share one lowering; mode changes only verified authority references and manifest metadata | proposal cannot promote itself |
| external manifest hash | envelope required | embedding a manifest's own hash creates recursion; every artifact is joined inside, manifest bytes are hashed by the bundle envelope | deterministic content address |
| refusing all information loss | conservative target support | Issue #49 requires safety/acceptance loss to fail closed; v0 also refuses representational loss rather than silently guessing | safe experimental default |

## 8. Remaining unknowns and next evidence

1. Exercise `reviewed_compilation_mode` against a store-produced accepted
   topology claim in the first end-to-end reconciler fixture.
2. Measure mapping size and change amplification on at least two real topologies.
3. Define versioned policy schemas before interpreting policy bodies or claiming
   semantic enforcement by the generic JSONL adapter.
4. Benchmark canonicalization and bundle memory at 1k/10k nodes; current evidence
   is functional, not performance evidence.

## 9. Quality checklist

- [x] Local benefit and present constraints explained.
- [x] `B/M/N/T`, cost bearer, and inversion boundary identified.
- [x] Structural and executable-test evidence kept separate from hypotheses.
- [x] Current, local, and cross-boundary counterfactuals compared.
- [x] Migration cost and false positives considered.
- [x] Severity and confidence reported independently.

## 10. Cross-issue integration correction

After Issues #51 and #54 introduced repository-owned policy contracts, the
compiler's earlier generic identity check became a harmful time-delayed local
optimum. Locally, requiring every opaque document to expose `policy_id` kept
the compiler independent of policy vocabulary. Across the Graph Engineering
Plane, however, the real contracts expose `verification_policy_id` and
`expansion_policy_id`; a valid policy was therefore refused while compiler-only
fixtures passed with documents no other component produced.

The expanded conditions were `B`: compiler plus verification/expansion
deployment, `M`: end-to-end contract preservation rather than compiler test
simplicity, `N`: all three experimental modules, and `T`: the first downstream
policy addition. Structural evidence was the incompatible identity fields;
execution evidence was a new compile test using both checked-in policy examples.
The burden fell on runtime integrators, who otherwise would have had to rewrite
valid documents into compiler-only shapes.

Counterfactual A kept the generic `policy_id` convention; B special-cased only
the two identity field names; C delegates shape and semantics to the canonical
#51/#54 validators and retains the explicit generic check only for budget,
which has no repository-owned contract yet. C was adopted. It adds compile-time
module coupling, but avoids a second validator and makes future semantic changes
fail closed at their existing owner. The correction is `mixed`
(`externalization` + `time-delayed`), `E=3, A=2, F=2, K=3, T=2`, severity
`12/15`, confidence `C2`. Remaining uncertainty is budget-policy evolution;
when a typed budget contract exists, it must replace the compiler's explicit
generic identity check through the same delegation pattern.
## Policy-aware analysis integration

最終横断監査で、実policy validatorをcompilerへ接続してもgraph analysis自体はpolicy非対応entry pointを使い、missing anchorやactor correlationがbundle reportから消える局所最適を検出した（E2/A2/F2/K2/T2=10、C3）。compilerは供給された実verification policyをtyped parseし、policy-aware graph lintへ渡すよう変更した。invalid documentのfail-closed判定は従来どおりpolicy validatorが所有し、analysisはdeterministic/heuristic分類をbundleへ保存する。

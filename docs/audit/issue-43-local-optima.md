# Issue #43 implementation local-optima audit

## Scope and outcome

- Scope: `execution_topology.rs`, the experimental v0 schema/examples, and the
  case/execution/runtime contract boundary document.
- System outcome: let graph designers exchange and criticize deployable shape
  without turning a proposal into accepted case state or freezing v0 as a
  stable wire promise.
- Evidence: structural diff and strict types; four passing Rust tests; ADR 0002
  and the existing stable `execution.plan.v1` boundary; current Git history.
- Verdict: no high-confidence harmful local optimum remains. One time-delayed
  drift signal is retained and bounded by executable examples.

## Evaluation conditions

| Variable | Local condition | Expanded condition |
|---|---|---|
| `B` | one parser/schema | case graph, compiler, runtime adapters, consumers |
| `M` | implementation simplicity and strict parsing | trust-boundary clarity, change cost, interoperability |
| `N` | new experimental module only | stable schema/toolchain may change after real integrations |
| `T` | P0 delivery | multiple incompatible v0 iterations and eventual promotion |

## Ranked candidates

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | JSON Schema plus Rust shape | strict wire documentation and dependency-free typed parsing | maintainers may update two representations | repeated v0 evolution | 6/15 | C2 | time-delayed, bounded |
| 2 | experimental namespace | avoids stable-contract review and migration now | adapters must opt into an unstable path | stable consumers | 3/15 | C2 | harmless-locality |

## Candidate 1: dual schema/type representation

### Facts, inference, hypothesis

- **Observed:** both artifacts deny unknown fields; both shipped examples parse
  through the Rust type; required arrays are required in serde as well as JSON
  Schema; a test joins the schema `$id` to the Rust constant.
- **Observed:** no JSON Schema validator/code generator dependency exists, and
  CLAUDE.md requires a measured ADR before adding one.
- **Inference:** a field change can require coordinated schema, type, example,
  validation, and test edits.
- **Hypothesis:** after several external integrations, omissions could survive
  if CI only exercises examples that do not use the changed branch.

### Local rationality and compensation halo

The direct typed implementation keeps runtime validation deterministic, small,
and auditable with the repository's existing serde stack. The compensation is
parallel schema maintenance, example round-trips, and explicit cross-reference
validation; repository maintainers bear that cost when the vocabulary changes.

### Boundary inversion

| Boundary | Current approach | Generated alternative | Advantage |
|---|---|---|---|
| module | no new toolchain; precise semantic checks | generator setup and generated-code review | current |
| feature | schema and parser can drift | one generated structural source | mixed |
| system | adapters need readable schema and stable diagnostics | generator-specific output couples consumers | current |
| lifecycle | repeated field changes amplify maintenance | generation may reduce repetition | alternative may win after evidence |

The earliest plausible inversion is lifecycle-scale repeated evolution, not the
current P0 module. No runtime/incident evidence confirms it yet.

### Counterfactuals and migration valley

- **A — keep current:** retain dual artifacts and executable examples; low
  immediate cost, possible future drift.
- **B — local improvement:** add representative fixtures and parity assertions
  when each new branch is introduced; low migration cost. This issue added ID,
  strict parsing, cross-reference, and two materially different fixtures.
- **C — cross-boundary generation:** select schema-to-Rust or Rust-to-schema,
  measure its dependency tree, write the ADR, migrate diagnostics, and preserve
  semantic validation. This temporarily creates two sources plus generator
  output and is not justified before real v0 usage.

Score: `E=1, A=2, F=0, K=1, T=2`, severity `6/15`, confidence `C2`.
Classification: `time-delayed`; intervention deferred until field-change or
integration history supplies an actual advantage reversal.

## Candidate 2 and false positives considered

Experimental isolation adds an adapter boundary, but broadening the time axis
strengthens rather than reverses its benefit: premature promotion would impose
compatibility and migration cost on every consumer. Intentional duplication of
case meaning was also checked and not found: work cells are references, and
case readiness/evidence/review rules are not reimplemented.

## Remaining unknowns and next evidence

1. Track files changed per topology vocabulary revision.
2. Run at least two external-runtime integrations before promotion.
3. Record whether schema/type drift is caught by fixtures or reaches review.
4. Re-evaluate code generation only if repeated changes raise amplification.

## Post-audit correction

An independent review found that serde defaults had allowed omission of arrays
the JSON Schema marks required. Those defaults were removed and an omission
counterexample was added. It also found that the small file-review example used
an authority edge without a typed summary handoff; the data edge now exists
beside the authority seam. The 1,000-item hierarchical reduction fixture is now
loaded by the contract test. These corrections reduce candidate 1's residual
severity from `6/15` to `4/15`; confidence remains C2.

# Issue #61 implementation local-optima audit

## Scope and outcome

- Scope: the real-runner adapters, isolation/capture harness, release policy,
  opt-in workflow, documentation, and retained two-provider smoke evidence.
- System outcome: real agent behavior is observable without turning provider
  output or subjective review into CaseGraphen acceptance.
- Evidence: harness tests plus the 2026-08-03 Codex/Claude durable smoke report.
- Verdict: the former fake-runner-only arrangement was a **time-delayed local
  optimum**. The replacement keeps deterministic CI cheap while moving actual
  provider behavior to an explicit release boundary.

## Evaluation conditions

| Variable | Previous local condition | Expanded audit condition |
|---|---|---|
| `B` | manifest and fake subprocess | fresh workspaces, two provider CLIs, release review and retained artifacts |
| `M` | deterministic, credential-free CI | reproducibility, provider identity, behavior regressions, cost and secret boundaries |
| `N` | one local test runner | opt-in provider adapters plus deterministic evaluators and human judgment |
| `T` | one test invocation | provider/model/Skill changes across releases |

## Facts, inferences, and hypotheses

- **Observed:** unavailable provider executables are reported with exit 3 and
  are never replaced by a fake runner.
- **Observed:** each scenario receives a fresh directory containing only its
  declared inputs and selected Skill tree.
- **Observed:** reports capture runner identity, hashes, timing, raw stream
  hashes/bytes, produced files, deterministic results, and provider-reported
  usage without serializing the environment.
- **Observed:** one real Codex and one real Claude run passed the deterministic
  review-seam assertions; their manual judgment was resolved in the durable
  report. The full matrix was not represented as completed.
- **Inference:** running provider calls in every ordinary CI build would reduce
  apparent release ceremony while externalizing nondeterminism, credential,
  latency, and cost failures to contributors.
- **Hypothesis:** provider-side telemetry formats will change; keeping usage as
  untrusted observations prevents that drift from changing acceptance truth.

## Candidate and compensation halo

| Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---|---|---|---|---:|---|---|
| fake-runner evidence presented as agent behavior | fast and deterministic | false confidence in Skill behavior | release lifecycle | 10/15 | C3 | time-delayed |
| mandatory provider calls in normal CI | immediate repeated evidence | secrets, cost, nondeterminism and provider availability | contributor/release boundary | 8/15 | C2 | displaced-cost candidate |

The earlier compensation halo was manual ad-hoc invocation with no common
identity, hashes, raw artifacts, or threshold policy. The burden fell on the
release reviewer precisely when comparing a regression mattered.

## Boundary inversion and counterfactuals

- **A — keep fake-only CI:** lowest cost, but no evidence of actual Skill use.
- **B — call providers in every CI run:** more samples, but makes external
  credentials and nondeterminism a merge prerequisite.
- **C — deterministic normal gate plus explicit release matrix (implemented):**
  preserves a fast gate and makes real behavior/cost/manual review visible.

At function level C is more complex; at product and lifecycle boundaries it is
the only option that does not confuse harness correctness with agent behavior.

## Score and decision

- `E=2`, `A=2`, `F=2`, `K=2`, `T=2`; severity `10/15`, confidence `C3`.
- Classification: `time-delayed` for fake-only evidence.
- No stable-promotion claim is made from the smoke. The full 20-run matrix and
  resolved judgments remain an explicit release condition, which prevents the
  new harness itself from becoming a checkbox optimum.

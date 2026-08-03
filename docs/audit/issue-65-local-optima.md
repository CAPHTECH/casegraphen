# Issue #65 implementation local-optima audit

## Scope and result

- Scope: the product architecture, command, Skill, contract, MCP, and evaluation claims added to `README.md`, plus its conformance hook.
- Outcome sought: a first-time user sees the product that is actually shipped without mistaking experimental/library/reference surfaces for stable operational guarantees.
- Evidence: README links and command snippets, `src/cli_usage.txt`, the four Skill trees, experimental README/ADRs, and a passing `python3 scripts/skill-conformance.py --check`.
- Verdict: the prior implementation-focused README was a **time-delayed documentation local optimum**; the new overview removes the mismatch while leaving detailed rules in their owning documents.

## Evaluation conditions

| Variable | Local condition | Expanded condition |
|---|---|---|
| `B` | stable ledger and worker CLI | whole product, including experimental graph plane and external adapters |
| `M` | concise README and low documentation maintenance | accurate discovery, trust-boundary clarity, and command reproducibility |
| `N` | edit README independently | link owning ADR/design/schema/Skill sources and gate command claims |
| `T` | one release | repeated experimental contract and product-surface changes |

## Observations and reasoning

- **Observed:** the former README omitted graph lint, three Skills, experimental schemas, compiler/reconciler/resource/simulation/redesign modules, and MCP reference status.
- **Observed:** it said only `space reason` accepted text despite graph lint supporting text.
- **Observed:** the revised README explicitly separates stable Case Graph, experimental Execution Topology, and untrusted Runtime Run Graph; it labels v0 and the stdio adapter accurately.
- **Observed:** it lists all four Skills by responsibility and links details instead of copying their procedures.
- **Observed:** README now runs through the same command/path/obsolete-claim checker as the Skills.
- **Inference:** the former brevity moved discovery and trust-boundary interpretation costs to new users and integrators.
- **Hypothesis:** a generated complete surface table may eventually replace the concise hand-written command list; present conformance evidence does not require that expansion.

## Candidate ranking and compensation

| Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---|---|---|---|---:|---|---|
| pre-plane README | stable narrative stayed short | users saw a different product and could over/under-estimate supported paths | product onboarding | 8/15 | C2 | time-delayed |

The compensation halo consisted of reading source, Skills README, schemas, and ADRs to reconstruct the real product. The burden fell on adopters and reviewers, and reappeared on every release.

## Boundary inversion and counterfactuals

| Boundary | Old README | Updated overview | Advantage |
|---|---|---|---|
| section | shorter | more concepts | old |
| repository | hidden modules/status | discoverable owners and links | updated |
| product adoption | reconstruct by source inspection | explicit support/stability labels | updated |
| lifecycle | drift unchecked | command/path statements gated | updated |

- **A — retain old README:** zero edit cost, persistent product mismatch.
- **B — duplicate every experimental contract in README:** maximum immediate detail, high drift and authority ambiguity.
- **C — concise architecture/surface overview plus owned links and conformance (implemented):** moderate length, low duplication, explicit instability.

## Score and decision

- `E=2`, `A=2`, `F=1`, `K=1`, `T=2`; severity `8/15`, confidence `C2`.
- Classification: `time-delayed`.
- No high-confidence replacement local optimum was found. The main risk—README becoming another contract owner—is limited by linking the detailed sources and checking executable/path claims.
- Unverified: operational MCP status and retained real-agent evidence must be updated by Issues #69 and #61 respectively; the README intentionally describes their current boundary rather than predicting completion.

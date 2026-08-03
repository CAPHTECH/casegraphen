# Issue #64 implementation local-optima audit

## Scope and outcome

- Scope: `scripts/skill-conformance.py`, `tests/skill_conformance.rs`, and the four shipped Skill trees.
- System outcome: every Skill keeps its distinct authority boundary while command, path, schema, and vocabulary claims follow the shipped product surface.
- Evidence: source structure, the four real Skill documents, mutation tests, and the passing `cargo test --locked --test skill_conformance` run.
- Verdict: the previous operate-only checker was a **time-delayed local optimum**. The replacement removes that boundary without turning all Skills into copies of the operate Skill.

## Evaluation conditions

| Variable | Previous local condition | Expanded audit condition |
|---|---|---|
| `B` | `casegraphen-operate` documentation | all shipped Skills, CLI/schema paths, and the release gate |
| `M` | low checker complexity and prevention of known operate drift | authority-boundary preservation and total documentation drift |
| `N` | edit one hard-coded Skill tree | change the shared checker, four role contracts, and mutation tests together |
| `T` | the next operate edit | repeated Graph Engineering Plane surface and schema evolution |

## Facts, inferences, and hypotheses

- **Observed:** the old checker had one `SKILL` root and traversed only `skills/casegraphen-operate`.
- **Observed:** the new inventory names all four shipped Skills, validates their Markdown references and CLI snippets, rejects known overclaims, and checks role-specific boundary statements.
- **Observed:** a table-driven test mutates one responsibility statement in each real Skill and proves that all four mutations fail.
- **Observed:** generated capability material remains only under `casegraphen-operate`; design, audit, and integrate are not forced to consume mutation/status vocabulary they do not own.
- **Inference:** the former low implementation cost externalized drift risk to users of three unguarded Skills and to release reviewers.
- **Hypothesis:** future non-Markdown references may require a typed manifest rather than more regular expressions; no such failing reference is currently observed.

## Candidate and compensation halo

| Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---|---|---|---|---:|---|---|
| operate-only conformance | one small checker and one generated file | three Skills could drift in commands, files, trust wording, and product claims | product/release boundary | 8/15 | C2 | time-delayed |

The compensation halo was manual review of design/audit/integrate documents and a release claim that could be green while those Skills were stale. The direct burden fell on Skill consumers and maintainers diagnosing agent behavior after release.

## Boundary inversion

| Boundary | Previous approach | Inventory-and-role approach | Advantage |
|---|---|---|---|
| function | fewer branches | a small inventory/role loop | previous |
| module | simple operate assumptions | shared parsing plus explicit role contracts | mixed |
| product | three unchecked Skills | all shipped Skills checked without shared mutation authority | replacement |
| lifecycle | each new Skill adds manual review debt | a new Skill must enter the inventory and declare its boundary | replacement |

## Counterfactuals and migration valley

- **A — keep operate-only:** no migration cost, but continued silent drift for three public Skills.
- **B — check every file with identical rules:** small change, but would create a false global contract and couple read-only/proposal Skills to operate vocabulary.
- **C — shared syntax/path checks plus role contracts (implemented):** modest checker complexity and explicit phrases to maintain; preserves role differences and gives deterministic mutation evidence.

The migration cost is limited to keeping a small role-contract inventory current. A future structured Skill manifest is a plausible alternative only if regular-expression maintenance becomes observable rather than hypothetical.

## Score and decision

- `E=2`, `A=2`, `F=1`, `K=1`, `T=2`; severity `8/15`, confidence `C2`.
- Classification: `time-delayed` with documentation-risk externalization.
- No new high-confidence local optimum was found in the implementation. Keeping generated CLI/status capability material scoped to operate avoids authority-vocabulary over-sharing.
- Unverified: actual fresh-agent behavior is intentionally outside this checker and remains Issue #61.

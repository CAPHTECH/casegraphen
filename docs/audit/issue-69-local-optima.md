# Issue #69 implementation local-optima audit

## Scope and outcome

- Scope: durable `ControlPlaneState`, authenticated/durable MCP session, `casegraphen-mcp-host`, host/resource docs and restart/E2E tests.
- System outcome: an external host can survive restart and expose real CaseGraphen projections and canonical graph decisions without becoming a scheduler, model runtime, or second acceptance implementation.
- Evidence: source structure, ADR/guide, passing `control_plane` and `mcp_stdio` tests, including forced acknowledgement failure and process restart.
- Verdict: the stateless reference adapter was a **harmless locality** for protocol demonstration but a **time-delayed local optimum** when presented as the only fleet boundary. The operational host removes that product gap while retaining the reference binary.

## Evaluation conditions

| Variable | Reference-local condition | Expanded operational condition |
|---|---|---|
| `B` | one stdio process lifetime | CaseGraphen store, external projections, restart/reconnect, operator reconciliation |
| `M` | minimal adapter and no persistence dependency | no duplicate delegated effect, explicit auth, canonical decision ownership |
| `N` | process-local maps | external binary plus reusable protocol state and existing library owners |
| `T` | one MCP session | crashes, acknowledgement ambiguity, repeated deployments and catalog growth |

## Facts, inference, hypotheses

- **Observed:** `casegraphen-mcp` remains stateless and fail-closed.
- **Observed:** the operational host requires an environment-sourced token, atomically journals pending/completed requests and notifications, and loads them after restart.
- **Observed:** a forced crash-equivalent between delegate return and durable acknowledgement leaves a pending marker; restart refuses `ambiguous_prior_effect` and the delegate call count remains one.
- **Observed:** real space status/frontier/review/revision resources use `NativeCaseStore` replay and the canonical evaluator; topology lint/compiler and runtime reconciliation call their existing modules.
- **Observed:** unsupported catalog operations refuse explicitly; the host does not implement readiness, review, gate, compilation, or completeness algorithms itself.
- **Inference:** persisting only completed responses would optimize the happy path while externalizing duplicate-effect risk to the ledger/operator after a crash.
- **Hypothesis:** multi-host active/active deployment would require a transactional shared journal or single-writer lease. The current guide deliberately constrains one private host instance, so no unsupported distributed guarantee is claimed.

## Candidate ranking

| Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---|---|---|---|---:|---|---|
| process-local replay as operational state | zero storage and recovery complexity | duplicate/forgotten effect and lost reconnect cursor after restart | process lifecycle | 11/15 | C3 | time-delayed |
| host-local reimplementation of decision rules | fewer library calls | rule drift and authority divergence | system integrity | 12/15 | C2 | rejected alternative |

The old compensation halo was client retries, manual state reconstruction, and custom embedding code. Its burden fell on operators and integrators exactly during failures.

## Boundary inversion and counterfactuals

| Boundary | Stateless reference | Durable external host | Advantage |
|---|---|---|---|
| function/session | smaller and simpler | journal/auth branches | reference |
| product integration | custom owner required | supported process and projections | host |
| crash/reconnect | state lost | replay or explicit ambiguity refusal | host |
| lifecycle | every adopter rebuilds host concerns | one documented external boundary | host |

- **A — keep only reference adapter:** correct demonstration, no operational product.
- **B — move daemon/persistence/rules into core:** one package but violates runtime ownership and duplicates decisions.
- **C — separate durable host that delegates canonical owners (implemented):** additional process/configuration and a single-writer deployment constraint, with a reversible stateless reference path.

## Score and decision

- Process-local operational state: `E=3`, `A=2`, `F=3`, `K=1`, `T=2`; severity `11/15`, confidence `C3`.
- Classification: `time-delayed`.
- No high-confidence replacement local optimum was found within the declared single-host boundary. The deliberate partial tool binding is safer than placeholder success; the product-surface inventory must keep supported versus catalogued operations explicit (Issue #63).
- Unverified: active/active persistence, network transport, and external identity-provider integration are non-goals, not silently satisfied guarantees.

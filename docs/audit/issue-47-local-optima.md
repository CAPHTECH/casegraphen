# Issue 47 implementation local-optima audit

## 1. Executive summary

- Scope: `casegraphen-audit`, its focused references, behavior eval, and install
  boundary.
- System outcome: diagnose static and planned-versus-reported graph behavior
  without duplicating graph/completeness rules or promoting runtime claims.
- Conclusion: no high-severity local optimum was found. One bounded
  externalization remains: run auditing requires a host integration to call the
  Rust completeness API because the shipped CLI has no runtime-reconcile
  command. The Skill fails closed instead of reimplementing that algorithm.
- Evidence limit: static code, fixture tests, and a tested fresh-process harness
  are available; no external-agent release run, production audit usage, runtime
  trace, Git trend, or organizational ownership is available.

## 2. System result and evaluation conditions

| Variable | Current condition | Expanded condition |
|---|---|---|
| `B` boundary | Skill, linter CLI, runtime-protocol library | installed consumer, runtime adapter, reconciler, review workflow |
| `M` metric | correct classifications and no duplicate rules | end-to-end audit availability, drift cost, and review safety |
| `N` change scope | Skill/references/tests/install | CLI, adapter, protocol, and report contract together |
| `T` time | experimental v0 and P0 behavior eval | multiple adapters and repeated post-run audits |

The local design optimizes trust-boundary safety: static facts come from
`graph lint`, completeness comes from `reconcile_runtime_reports`, and the
Skill never fills a missing integration with agent arithmetic.

## 3. Evidence

| Surface | Source | Observation | Limit |
|---|---|---|---|
| Structural | `skills/casegraphen-audit/` | Only `graph lint` is executable; run completeness names the one Rust API and forbids local counting/retry reconstruction. | Static evidence. |
| Runtime | `tests/casegraphen_audit.rs`, `tests/fresh_agent_eval.rs` | Real CLI retained classifications; canonical API owns the 199/200 oracle; manifest/isolation/capture work without a model. | Fixture and harness conformance, not fresh-agent behavior. |
| Evolution | `install.sh`, install smoke | Three Skills and canonical runtime schema install in both supported homes. | No longitudinal history. |
| Meaning/ownership | reporting boundary reference and runtime protocol | Identity/model/context are explicitly runtime-declared and untrusted; completeness remains diagnostic. | No external adapter owner observed. |

## 4. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | API-only run reconciliation | Prevents a second completeness algorithm and keeps issue 47 out of adapter scope | Consumer needs a host integration before run audit is available | installed standalone consumer | 6 | C2 | `externalization` |

## 5. Candidate LO-47-1: API-only run reconciliation

### Facts, inference, and hypothesis

- **Observed:** static audit has a shipped `casegraphen graph lint` command.
- **Observed:** run audit requires a host call to
  `casegraphen::runtime_protocol::reconcile_runtime_reports`; if unavailable,
  the Skill stops instead of manually deriving completeness.
- **Observed:** the behavior eval calls that function and obtains 200 expected,
  199 reports, one missing report, and `complete=false`.
- **Inference:** rule ownership is safer than embedding report counting in the
  Skill, but integration work is shifted to adapter/host owners.
- **Hypothesis:** repeated user-level audits will justify a stable read-only CLI
  or control-plane projection of the same API. No usage trace demonstrates that
  demand yet.

### Local rationality and compensation halo

- Local purpose: make false completion impossible without introducing a second
  decision rule.
- Direct beneficiaries: protocol maintainers and reviewers relying on one
  completeness meaning.
- Still-valid constraint: issue 47 is an audit Skill; generic ingest/control
  plane work belongs to later integration scope.

| Local decision | Boundary effect | Compensation | Cost bearer | Frequency/evidence |
|---|---|---|---|---|
| Expose completeness as Rust API, not Skill arithmetic | installed agent cannot invoke it directly | host adapter must validate, build expectation, and call the API | runtime integrator | static API boundary; no production frequency |
| Fail closed when result is absent | partial audit instead of fabricated result | operator supplies canonical output later | audit operator | specified and behavior-tested |

### Boundary inversion

| Boundary | Current approach | Shared read-only projection | Advantage |
|---|---|---|---|
| Function/module | direct typed call, no serialization ambiguity | extra CLI/report surface | current |
| Repository integration | behavior tests call the owner directly | simpler subprocess integration | current/neutral |
| Installed consumer | run audit may stop | immediately usable canonical reconciliation | alternative |
| Lifecycle | each adapter writes a thin caller | one maintained CLI/control-plane projection | alternative after multiple callers |

- Minimum inversion boundary: consumer outside a Rust host integration.
- Inverting metric: end-to-end audit availability and total adapter work.
- Inverting time: when a second runtime adapter needs the same projection.

### Counterfactuals

- **A — current:** retain the API-only boundary and explicit halt. Lowest scope
  and no semantic duplication; run audit depends on a host.
- **B — local improvement:** bundle an ad-hoc script that counts JSON reports.
  It appears convenient but creates exactly the duplicate completeness rule
  prohibited by the contract; reject this option.
- **C — cross-boundary change:** add a read-only CLI/control-plane command that
  parses typed inputs and calls `reconcile_runtime_reports`. It adds parser,
  output schema, compatibility tests, and release surface. Migration is small
  because the Rust API remains the owner; rollback is removal of the projection
  before it is declared stable.

Scores: `E=2`, `A=1`, `F=1`, `K=1`, `T=1`, **Severity 6/15**,
**Confidence C2**. Verdict: `externalization`, acceptable during experimental
v0 with a clear trigger for reevaluation. It is not a high-severity candidate.

## 6. Rejected candidates

| Target | Signal | Why inversion was not established |
|---|---|---|
| Four evidence-class terms in Markdown and fixture | duplicated vocabulary | This is the audit presentation contract itself, not a duplicate graph/completeness rule; a behavior test detects accidental collapse. |
| Installed copy of runtime JSON Schema | duplicate file | It is copied byte-for-byte from the canonical schema during install and checked with `cmp`; no repository fork exists. |
| Separate static/run references | fragmented instructions | Progressive disclosure reduces context and both are directly linked from `SKILL.md`; no contradictory rule was observed. |

No retry, acceptance, scheduler, or manual-reconciliation compensation was
introduced.

## 7. Unverified gaps and next evidence

### Fixture conformance versus release evaluation

CI validates the ten-scenario manifest and exercises the harness with a
non-model runner. The 199/200 expected values are checked against
`reconcile_runtime_reports`, so the harness compares agent output with a
canonical oracle instead of implementing report counting. Schema and graph
checks likewise call their existing owners where available. These checks prove
the evaluator wiring, not that a fresh agent follows the audit Skill.

The opt-in release path requires an explicit external `--runner-json`, creates
one temporary workspace/process per scenario, captures untouched stdout,
stderr, and workspace files, and marks semantic review as `manual_required`.
Because a configured runner may load provider/user context outside that
workspace, the harness observes process/workspace isolation but cannot attest
context independence. This bounded externalization scores severity 5/15,
confidence C2; release reports must state the runner and manual evidence.

1. Run the opt-in static, 199/200, and verifier-correlation scenarios with an
   external fresh agent; current deterministic evals do not prove model
   generalization.
2. Record how many adapters need a callable completeness projection. A second
   independent caller is the trigger to evaluate counterfactual C.
3. Obtain production audit traces before asserting latency/cost mismatch,
   verifier false positives, or non-convergent expansion frequency.
4. Measure co-change between the protocol, audit references, and integrations
   after several releases; current change amplification is unknown.

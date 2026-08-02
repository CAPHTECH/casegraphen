# Issue 46 implementation local-optima audit

## 1. Executive summary

- Scope: `casegraphen-design`, its progressive references, install surface, and
  deterministic behavior fixtures.
- System outcome: turn a problem statement into an inspectable topology proposal
  without creating a second acceptance rule or mutating the case ledger.
- Conclusion: no high-severity local optimum was found. One time-delayed candidate
  remains: Markdown mapping proposals are cheap and deliberately unstable now,
  but would externalize parsing and migration cost if runtimes begin consuming
  them as contracts.
- Evidence limit: this audit has static structure, fixture tests, and a tested
  fresh-process harness, but no external-agent release run, production runtime
  trace, Git-history trend, or fresh-model behavior sample.

## 2. Evaluation conditions

| Variable | Current condition | Expanded condition |
|---|---|---|
| `B` boundary | one design Skill and one proposal directory | compiler, runtime adapter, reconciler, and acceptance ledger |
| `M` metric | concise instructions and valid lint output | semantic drift, deterministic interchange, and safe evolution |
| `N` change scope | Skill, references, fixtures, install script | topology schema, linter, mappings, and runtime protocol together |
| `T` time | experimental v0 delivery | repeated topology revisions and multiple runtime adapters |

The current constraints are proposal-only operation, experimental contracts,
and reuse of the shipped typed schema and linter rather than reproducing rules.

## 3. Evidence

| Observation surface | Evidence | Result and limitation |
|---|---|---|
| Structural | `skills/casegraphen-design/SKILL.md`, `references/` | The only executable CaseGraphen command is `graph lint`; policy and deployment metadata are explicitly unreviewed. Static evidence only. |
| Runtime | `tests/casegraphen_design.rs`, `tests/fresh_agent_eval.rs` | The shipped binary linted fixtures; manifest and isolated-runner plumbing execute without a model. This proves fixture/oracle and harness conformance, not Skill generalization. |
| Evolution | `scripts/install-smoke-test.sh`, `scripts/skill-conformance.py --check` | Both Skills install and the existing generated CLI surface remains current. No longitudinal change-frequency evidence. |
| Meaning/ownership | topology v0 schema and design references | Typed topology and acceptance state remain separate; Markdown mappings have no stable machine contract. Ownership beyond this repository is unknown. |

## 4. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Markdown mapping proposals | Avoids prematurely stabilizing CaseGraphen genesis/plan compilation | Adapter authors may invent parsers and incompatible conventions | second independent runtime consumer | 5 | C1 | `time-delayed` |
| 2 | Fresh process as a proxy for fresh context | Portable runner contract without provider coupling | external runner may load user/global memory beyond the isolated workspace | runner with implicit persistent context | 5 | C2 | `externalization` |

## 5. Candidate LO-46-1: Markdown mapping proposals

### Facts, inference, and hypothesis

- **Observed:** `SKILL.md` requires `genesis.mapping.proposal.md` and
  `execution-plan.mapping.proposal.md`; the stale-revision fixture verifies
  revision preservation by inspecting Markdown text.
- **Observed:** `contracts-and-outputs.md` labels the mappings unreviewed and
  says not to invent a stable schema.
- **Inference:** the representation is locally rational while the compiler
  vocabulary is experimental, because it prevents an accidental second stable
  contract.
- **Hypothesis:** if two runtime adapters consume these files automatically,
  each may implement a text convention and shift compatibility cost to adapter
  maintainers. No such consumers were observed.

### Local rationality and compensation halo

- Local purpose: communicate an auditable mapping without writing accepted
  state or freezing an immature compiler contract.
- Beneficiaries: Skill authors and early topology reviewers.
- Still-valid constraint: issue 46 is proposal generation, not a graph compiler.

| Local decision | Boundary effect | Compensation | Cost bearer | Evidence |
|---|---|---|---|---|
| Keep mappings human-readable and unstable | no deterministic interchange contract | humans review text; fixtures search sentinel phrases | future integrator, if automation appears | static only |

There is currently no adapter/parser compensation halo in the repository, so
the candidate is time-delayed rather than an existing externalization.

### Boundary inversion

| Boundary | Current representation | Typed mapping alternative | Advantage |
|---|---|---|---|
| Skill | small, reviewable, explicitly provisional | schema design and validation overhead | current |
| Feature | adequate for proposal handoff | safer automated compilation | conditional |
| System | conventions may diverge across adapters | one typed interchange boundary | alternative after multiple consumers |
| Lifecycle | cheap until the first consumer | migration is cheaper if introduced before conventions spread | alternative when adoption begins |

- Minimum inversion boundary: a second independent machine consumer.
- Inverting metric: total integration and migration cost, not initial file count.
- Inverting time: when mapping compilation becomes supported product behavior.

### Counterfactuals and migration valley

- **A — current:** retain Markdown proposals. Lowest present cost; monitor for
  machine consumers and do not advertise the mappings as contracts.
- **B — local improvement:** add more Markdown conventions. This helps one
  adapter but increases the risk of an undocumented de facto schema.
- **C — cross-boundary change:** define a typed mapping/compilation contract
  jointly with the reconciler and runtime adapter. It requires schema review,
  compatibility tests, and a period where Markdown and typed mappings coexist;
  rollback remains possible while Markdown is retained as a projection.

Scores: `E=1`, `A=1`, `F=0`, `K=1`, `T=2`, **Severity 5/15**, **Confidence C1**.
Verdict: `time-delayed`. Reclassify only when an actual machine consumer or
change history supplies a second observation surface.

## 6. Compensation and non-candidates

No cross-cutting retry, fallback, duplicate scheduler, or manual acceptance
compensation was introduced. Two apparent duplication signals were rejected:

| Target | Initial signal | Why it is not a local optimum |
|---|---|---|
| Metric/finding assertions in behavior tests | expected values resemble linter knowledge | They are independent executable oracles against the shipped CLI, not a second implementation; the Skill contains no thresholds or lint algorithms. |
| Temporary-directory helper in the Rust test | small utility duplication | It is test-local lifetime isolation and adds no production dependency or semantic rule. |
| Separate Skill references | repeated graph concepts | Progressive disclosure limits context while every normative enum and validation rule points back to the v0 schema and real linter. |

## 7. Unverified gaps and next evidence

### Fixture conformance versus release evaluation

Normal CI runs `scripts/fresh-agent-eval.py --check-manifest` and a non-model
fake runner. It proves that all ten scenarios exist, task artifacts and Skills
are copied into a fresh temporary workspace, raw output is captured, and
deterministic evaluators are wired. It does **not** execute an agent or establish
behavior quality. An opt-in release evaluation supplies `--runner-json` and an
output directory, starts a new external process per scenario, then records
deterministic results and leaves semantic judgments as `manual_required`.

The workspace excludes the repository and prior scenario output, but the
external runner inherits credentials/configuration needed to start. Therefore
fresh process/workspace is observed; provider context freshness is not. This is
an explicit externalization, severity 5/15, confidence C2, and must remain a
manual release-evidence qualification rather than an attested fact.

1. Run the opt-in scenarios with fresh-context agents and measure correct
   artifact production; deterministic fixtures do not establish model
   generalization.
2. Track whether runtime adapters begin parsing mapping Markdown. That is the
   trigger for a typed compiler contract, not the mere existence of proposals.
3. After several releases, inspect co-change history between the Skill, topology
   schema, linter, and adapters to determine whether a hidden second rule source
   has emerged.
4. Production cost, latency, and organizational ownership remain unobserved and
   must not be inferred from these tests.

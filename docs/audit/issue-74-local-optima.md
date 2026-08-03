# Issue 74 implementation local-optima audit

## 1. Executive summary

- Mode: `intervention`, after the fresh-agent harness and workflow tests passed.
- Scope: provider workflow, process environment construction, runner identity,
  evidence retention, release policy, static conformance, tests, and operator guide.
- System outcome: each real provider receives only its own credential, missing
  authority is explicitly unavailable, and retained evidence is tied to a
  policy-owned exact package/version pin.
- Result: the audit found and fixed two time-delayed local optima. Runner pins
  moved into the release policy so the workflow, harness, and checker cannot
  silently select different identities. Version probing was also stripped of
  provider credentials so a compromised `--version` path cannot leak them into
  retained identity evidence.
- Evidence constraint: tests use fixtures and static workflow inspection; no
  real GitHub-hosted provider run or platform secret-redaction log was available.

## 2. Evaluation conditions

| Variable | Initial boundary | Expanded boundary |
|---|---|---|
| `B` | one matrix job | workflow event -> step secret -> child environment -> generated files -> uploaded evidence |
| `M` | provider command starts | least authority, explicit unavailability, reproducible identity, non-disclosure |
| `N` | edit workflow `env` | policy, workflow, harness, scanner, conformance, docs, tests |
| `T` | current Codex/Claude run | runner upgrades, provider additions, failed/hostile runs, release audit |

## 3. Evidence

| Plane | Evidence | Constraint |
|---|---|---|
| Structure | conditional workflow steps, `provider_environment`, release policy pins, conformance checker | repository configuration only |
| Execution | nine `fresh_agent_eval` tests, including missing credential, env isolation, and generated-file leak | fake/custom runners, not hosted providers |
| Evolution | pins previously repeated as independent literals; now policy-owned and checked | no package upgrade history yet |
| Meaning | `credential_unavailable`, `declared_package_identity`, observed version and `version_matches` are distinct | npm install receipt is workflow evidence, not cryptographic provenance |

## 4. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Both provider secrets at job scope | one matrix step | every provider child and setup step receives unrelated authority | process trust boundary | 12 | C3 | mixed; fixed |
| 2 | Independent pin literals | simple workflow | upgrades can install, assert, and report different identities | release lifecycle | 8 | C3 | time-delayed; fixed |
| 3 | Credential-bearing version probe | reused provider environment | pre-evaluation command can disclose authority into summary | retained-evidence boundary | 9 | C2 | mixed; fixed |
| 4 | Exact-value secret scanning | deterministic and low false-positive logic | encoded, transformed, or provider-held secrets are not proven absent | adversarial disclosure boundary | 6 | C2 | bounded limitation |

## 5. Candidate details

### C1 — Matrix convenience widened provider authority

**Facts.** The previous job-level environment exposed both provider keys to
dependency installation, builds, and both matrix lanes. The workflow now has no
job-level secrets and uses two mutually exclusive execution steps. The harness
independently removes all secret-like variables and restores only the selected
profile credential. Tests observe only `OPENAI_API_KEY` in a Codex child even
when Anthropic and GitHub credentials exist in the parent.

The local design reduced YAML duplication. The cost was borne by every process
in the job and by incident responders unable to prove least authority. A)
retain job scope and trust providers; B) step-only secrets; C) step-only plus
child-process filtering (chosen). `E=3, A=2, F=2, K=3, T=2`, severity 12,
confidence C3, classification `mixed`.

### C2 — Runner identity had multiple owners

**Facts.** Package, expected version, and documentation initially repeated
literals. That is locally readable but a later one-file upgrade could install
one version while the harness records another. `runner_pins` in the release
policy is now the semantic owner. The harness refuses caller declarations that
do not exactly match it, and workflow conformance derives its assertions from
the same policy. Workflow literals remain necessarily duplicated because
GitHub expressions cannot directly install from repository JSON; the checker
is the compensation boundary.

Counterfactuals: A) independent literals; B) generate workflow YAML (larger
tooling ownership); C) policy owner plus static cross-check (chosen). `E=2,
A=1, F=2, K=2, T=1`, severity 8, confidence C3, `time-delayed`.

### C3 — Identity discovery inherited unnecessary authority

**Facts.** `runner_identity` originally probed `--version` with the selected
provider environment. That made the shortest implementation reuse a helper,
but a compromised executable could print the credential into the unredacted
identity object. The probe now receives a secret-free environment; only actual
scenario execution receives the selected credential. `E=2, A=2, F=2, K=2,
T=1`, severity 9, confidence C2, `mixed`.

### C4 — Scanner scope is intentionally bounded

The harness redacts exact parent secret values from captured streams and
structured evaluator details, scans generated bytes for those exact values,
and withholds the entire workspace on a match. It cannot prove absence of
encoded or transformed values, provider-side storage, or an unknown secret not
named by a secret marker. Calling this comprehensive DLP would invert at the
adversarial boundary. Documentation therefore describes exact configured
credential-value protection, and publication still requires review. This is a
bounded limitation rather than a remediated global guarantee.

## 6. Compensation halo and false-positive guards

The policy-to-workflow conformance check is intentional compensation for a
declarative platform that cannot consume the JSON pin directly at expression
evaluation time. It runs in static analysis and has a negative fixture for
shared job secrets. Manual-only triggering plus read-only job permissions is a
second independent guard against untrusted PR authority. A provider lane that
lacks credentials or has the wrong installed version produces exit 3 and no
scenario results; it is never replaced by the fake runner.

## 7. Remaining evidence

| Priority | Evidence | Uncertainty | Method |
|---:|---|---|---|
| 1 | one hosted run per provider | GitHub event/secret behavior and observed version shape | manual workflow dispatch, retain summary |
| 2 | adversarial encoding fixture | transformed-secret disclosure | add explicit patterns only when threat model identifies them |
| 3 | provider upgrade exercise | pin update amplification and npm availability | release rehearsal with policy-first change |

Quality checklist: local benefit and burden owners are named; observations and
limits are separated; structural/execution/evolution evidence is used;
counterfactuals include migration cost; fixed findings and bounded limitations
are distinct; no production behavior is inferred from fixture-only evidence.

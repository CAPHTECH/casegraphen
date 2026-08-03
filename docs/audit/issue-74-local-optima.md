# Issue 74 implementation local-optima audit

## 1. Executive summary

- Mode: `intervention`, repeated after the authoritative authentication
  requirement changed from provider API keys to authenticated CLI sessions.
- Scope: provider workflow, self-hosted runner ownership, CLI auth preflight,
  child environment, runner identity, evidence retention, release policy,
  conformance, tests, and operator guide.
- System outcome: fresh-agent evaluation uses only pinned Codex/Claude CLIs
  with pre-authenticated disk-backed sessions. No provider API key or GitHub
  secret is injected into evaluation.
- Result: the prior per-step API-key design was locally safe relative to a
  shared job environment, but optimized the wrong authority model. It was
  removed. Provider-specific self-hosted labels, classified output-free CLI
  status probes, an allowlisted child environment, disk-leak canaries, and
  structured `cli_session_unavailable` form the harness boundary. Promotion
  additionally requires a broker-signed run/host attestation.
- Evidence constraint: fixture tests prove process and protocol behavior, but
  no authenticated self-hosted GitHub run was executed here.

## 2. Evaluation conditions

| Variable | Superseded boundary | Expanded boundary |
|---|---|---|
| `B` | hosted job -> step API key -> CLI | runner provisioning -> disk session -> status probe -> child process -> retained evidence |
| `M` | key isolation | correct auth mechanism, least environment authority, identity reproducibility, non-disclosure |
| `N` | workflow `env` | release policy, runner labels, harness, auth probes, scanner, conformance, docs, tests |
| `T` | one run | session expiry, runner reprovisioning, CLI upgrades, hostile output, release audit |

## 3. Evidence

| Plane | Evidence | Constraint |
|---|---|---|
| Structure | provider-specific matrix labels, `cli_session_environment`, auth-status commands, policy pins | static repository state |
| Execution | thirteen harness tests: auth classification, missing session, env/socket stripping, disk/env leak withholding, permission surface, workflow mutations | fake CLI, not a broker-attested account run |
| Evolution | API-key implementation removed after authority requirement clarification; pins policy-owned | no self-hosted runner upgrade history |
| Meaning | auth mode, classified non-API session, declared package, observed version, broker attestation, and retention policy are separate facts | broker/OS provisioning remains external |

## 4. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Per-step API-key authentication | familiar hosted CI | violates required authenticated-CLI execution semantics | product authority boundary | 13 | C3 | mixed; removed |
| 2 | One generic authenticated runner | less runner administration | either provider can inherit the other's session/configuration | host trust boundary | 11 | C2 | mixed; avoided |
| 3 | Retaining auth probe output | easier diagnostics | account metadata can enter artifacts | evidence boundary | 9 | C2 | mixed; avoided |
| 4 | Checkout credential persistence | standard action default | model can recover workflow authority from Git config | workspace boundary | 10 | C3 | mixed; fixed |
| 5 | Claude permission bypass and ambient user config | fewer eval refusals | model tools can inspect session/config beyond task | agent tool boundary | 12 | C3 | mixed; fixed |
| 6 | Independent runner-pin literals | readable workflow | install/assert/report identities drift | release lifecycle | 8 | C3 | time-delayed; fixed |
| 7 | Exact-value secret scan | deterministic guard | transformed or unknown disclosure is not proven absent | adversarial boundary | 6 | C2 | bounded limitation |
| 9 | Promoting the runner's own session assertion | compact evidence | caller can self-assert provenance | release authority boundary | 12 | C3 | mixed; fixed |

## 5. Candidate details

### C1 — Correct isolation around the wrong authentication primitive

The earlier implementation moved each API key from job scope to its provider
step and filtered unrelated credentials from the child. That was locally
rational under an API-key assumption, but the authoritative product contract
requires authenticated Claude Code and Codex CLI sessions and prohibits API-key
use. At the product boundary, better key isolation could not make the mechanism
valid. A) shared keys; B) isolated keys; C) no keys, authenticated CLI sessions
on provisioned runners (chosen). `E=3, A=2, F=3, K=3, T=2`, severity 13,
confidence C3, classification `mixed`.

### C2 — Session isolation belongs to runner provisioning

A single self-hosted machine with both sessions is operationally cheaper, but
the child can observe `HOME` because that is how the CLI reaches its session.
Process filtering cannot make a disk session CLI-readable but model-tool-
unreadable under the same OS authority. The child now receives an allowlist
without config overrides or agent sockets, and a disk canary makes retained
copies fail/redact. These are retention controls, not credential isolation.
The workflow therefore requires distinct
provider labels (`casegraphen-codex-cli-session` and
`casegraphen-claude-cli-session`) under a common fresh-agent label. Runner
operators own label assignment and host access. This prevents accidental
co-location by workflow contract, though the GitHub administrator remains a
trusted provisioning boundary. `E=3, A=1, F=2, K=3, T=2`, severity 11,
confidence C2.

### C3 — Auth status is a decision, not evidence payload

`codex login status` and `claude auth status --json` may contain provider or
account diagnostics. Keeping the output would improve debugging while shifting
privacy review to artifact consumers. The harness parses it only in memory:
Claude stdout must name a known non-API `authMethod`; Codex stdout/stderr are
checked in memory for the known ChatGPT session line because the real CLI may
emit status on stderr. It retains only classification, exit status, auth mode, command
hash, and `probe_output_retained: false`. Exit zero with API-key, malformed, or
unknown output fails closed. `E=2, A=2, F=2, K=2, T=1`, severity 9, confidence
C3.

### C4 — Environment cleaning did not cover Git configuration

Removing token-like environment entries is locally sufficient only if process
environment is the sole credential transport. `actions/checkout` persists a
workflow credential in Git configuration by default, broadening authority to a
model that can inspect the checked-out repository. Both checkouts now set
`persist-credentials: false`. The authenticated provider job performs no
checkout at all; hosted prepare and aggregate checkouts disable persistence.
Workflow conformance rejects a build or checkout added to the provider job.
The provider job also refuses non-`main` dispatch refs and names a
provider-specific GitHub Environment intended to carry deployment protection,
so an unreviewed branch cannot reach the session-bearing job. Repository tests
verify the environment binding but cannot prove that required reviewers or
deployment-branch rules are configured in GitHub.
`E=3, A=2, F=2, K=2, T=1`, severity 10, confidence C3.

### C5 — CLI session and unrestricted model tools shared one account boundary

The former Claude profile used `bypassPermissions`, which maximized scenario
completion locally but allowed model-selected tools to inspect the dedicated
runner account, including session/configuration paths. Claude now uses
`acceptEdits`, only Read/Write/Edit tools, project-only settings, no slash
commands, and strict ambient MCP exclusion. Codex uses workspace-write plus
ephemeral/ignore-user-config controls. This may turn some scenarios into honest
runner failures; promotion cannot trade session safety for completion. `E=3,
A=2, F=2, K=3, T=2`, severity 12, confidence C3.

### C6 — Runner identity has one policy owner

`runner_pins` owns exact package/version, authentication mode, and runner label.
The harness rejects mismatching identity arguments; workflow conformance checks
the label/auth/version surface against the same policy. Workflow literals are
necessary platform configuration and are guarded duplication rather than a
second decision owner. `E=2, A=1, F=2, K=2, T=1`, severity 8, confidence C3.

### C7 — Retention scanning remains intentionally bounded

Real-provider children receive no secret-like environment values. The harness
still redacts/scans exact parent secret values as defense in depth and withholds
a generated workspace on a match. This cannot prove absence of encoded values,
unknown disk-session data, or provider-side retention. Documentation does not
claim comprehensive DLP; manual evidence review remains required.

### C8 — Build convenience shared the authenticated host boundary

The first CLI-session workflow checked out source, installed Python packages,
and built Rust code on the same self-hosted account that held the CLI session.
That was locally simple but gave repository and dependency build code ambient
access before the evaluator's environment filtering began. The workflow now
builds a short-lived evaluator bundle in an uncredentialed hosted `prepare`
job. The provider-specific runner only downloads the bundle and executes it;
all external Actions are commit-SHA pinned. Workflow inputs are passed through
quoted step environment values, and the evaluator binary is named by an
absolute `$GITHUB_WORKSPACE` path. `E=3, A=2, F=2, K=3, T=2`, severity 12,
confidence C3, classification `boundary inversion`.
The toolchain gate reads the explicit `with.toolchain` value when the action is
SHA-pinned and now fails if that input disappears; a SHA alone is executable
provenance, not a Rust-version declaration.

### C9 — Session provenance requires authority outside the provider run

A summary can record what its harness observed, but cannot independently prove
the host/session boundary that produced itself. Promotion now requires a
provider-specific HMAC over the summary hash, random run challenge, classified
session, opaque runner identity, and brokered boundary. A root/service broker
whose signing key is unreadable by the evaluation account independently reruns
the safe auth classification before signing. Missing, substituted, or caller-
only attestations fail closed. `E=3, A=2, F=2, K=3, T=2`, severity 12,
confidence C3, classification `mixed`.

[Hypothesis] Repository tests verify cryptographic binding and failure modes,
not that a deployed broker account/key ACL truly prevents evaluation-agent
access. That remains an external provisioning assertion requiring host audit.

## 6. Compensation halo and false-positive guards

Self-hosted runner administration is a deliberate cost of the CLI-session
contract, not an accidental regression to hosted CI. In-job CLI installation
is rejected because it would separate the executable under test from the
operator-authenticated pinned executable. Static analysis rejects secret/API-
key references, hosted provider execution, missing provider labels, unpinned
versions, and omitted `--auth-mode cli-session`. The aggregate job remains
hosted because it consumes retained files and needs no provider session.
The conformance check structurally extracts the provider/runner-label pairs;
merely mentioning both labels or swapping them between providers is refused.
It also refuses direct workflow-input interpolation in shell blocks, relative
evaluator paths, build/install/checkout on the session runner, and floating
Action references. Non-`main` provider dispatch and removal of the
provider-specific protected Environment are refused as well.

## 7. Remaining evidence

| Priority | Evidence | Uncertainty | Method |
|---:|---|---|---|
| 1 | one broker-attested run per labeled runner | actual session lifetime and broker integration | retain summary/attestation, never probe output/key |
| 2 | runner provisioning audit | labels truly map to isolated accounts/configuration | inspect host policy without copying session material |
| 3 | session expiry exercise | unavailable state remains fail-closed during a matrix run | revoke/logout on disposable runner and dispatch |
| 4 | GitHub Environment configuration | the named environments actually require reviewers and restrict deployment branches | inspect repository environment protection through GitHub administration/API |

Quality checklist: superseded local rationality is recorded rather than hidden;
authority owner and burden owner are named; observations and hypotheses are
separate; structural/execution/evolution evidence is used; counterfactuals and
residual trust are explicit; no global optimum is claimed.

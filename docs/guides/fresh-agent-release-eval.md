# Fresh-agent release evaluation

The checked-in ten-scenario harness is deterministic infrastructure, not
evidence that a real agent followed a Skill. A release evaluation becomes
evidence only when a `summary.json` from a real provider run is retained with
its raw outputs and reviewed manual judgments.

## Declared matrix

The release matrix is Codex and Claude over all ten scenarios in
`evals/fresh-agent/scenarios.v0.json`. The repository ships explicit
`--runner-profile codex` and `--runner-profile claude` adapters. Missing
executables produce `provider_unavailable`; an installed CLI without an active
CLI session produces `cli_session_unavailable`. Both exit 3 and are never
replaced with the fake test runner. Each scenario runs in a new temporary
directory containing only its task, declared artifacts, and selected Skill.

Run a provider explicitly:

```sh
python3 scripts/fresh-agent-eval.py \
  --runner-profile codex \
  --auth-mode cli-session \
  --runner-package-identity '@openai/codex@0.146.0' \
  --expected-runner-version 0.146.0 \
  --model <release-model-id> \
  --budget-usd 25 \
  --timeout 900 \
  --casegraphen-bin /absolute/path/to/casegraphen/target/release/casegraphen \
  --output-dir artifacts/fresh-agent/codex
```

Use `--runner-profile claude --auth-mode cli-session` for Claude. API keys and
GitHub secrets are not accepted as provider authentication. Before execution,
the harness runs the pinned CLI's own non-interactive status command (`codex
login status` or `claude auth status --json`) and retains only success/failure
and the probe command hash—never its output or account metadata. A zero exit
status alone is insufficient: Claude JSON must report `loggedIn: true` with a
policy-allowed non-API `authMethod`; Codex stdout/stderr are checked only in
memory for the known ChatGPT session line. API-key, malformed, or output with
no allowed session classification fails closed.

Provider evaluation runs only on manually provisioned self-hosted runners with
provider-specific labels. The Codex runner must already have the pinned Codex
CLI and an authenticated Codex CLI session; the Claude runner has the
corresponding Claude CLI/session. An uncredentialed hosted `prepare` job checks
out the repository, builds the canonical evaluator, installs evaluation-only
Python dependencies into an artifact directory, and publishes one immutable
evaluation bundle retained for the same 90-day review window as provider
evidence. The authenticated runner only downloads that immutable
artifact; it performs no checkout, dependency installation, or build. External
Actions are pinned to commit SHAs. The workflow does not install provider CLIs
or inject credentials. It is manual-only and has read-only repository
permission, so fork and untrusted pull-request events cannot start an
authenticated lane. The provider job also accepts only `refs/heads/main` and
enters a provider-specific protected GitHub Environment. Runner operators must
configure required reviewers, restrict label assignment, and restrict host
access accordingly. Provider/model inputs reach shell argv through quoted step
environment values rather than direct expression interpolation.

The harness keeps only `HOME` and a small process-variable allowlist so the
disk-backed session remains reachable. CLI config overrides, agent sockets,
Git credential configuration, and every token/secret/password/API-key-like
environment entry are removed before auth preflight and model execution. It never
serializes environment variables, redacts exact parent secret values from
stdout/stderr and evaluator details, and scans generated workspace bytes for
those exact values before retention. A match withholds the workspace and fails
the scenario. This is a retention boundary, not a claim to detect encoded or
transformed disclosures. The policy-declared package identity, expected and
observed versions, auth mode/status, and command hashes remain in `summary.json`.
Codex runs with workspace-write sandboxing, ephemeral state, and user config
ignored. Claude runs in `acceptEdits` mode with only Read/Write/Edit tools and
project settings; permission bypass, user settings, slash commands, and ambient
MCP configuration are disabled. These controls reduce model access to session
material; they cannot make a session file readable by the CLI but unreadable by
another process with the same OS authority.

Stable promotion therefore requires a stronger host boundary. Run each
provider under a dedicated OS evaluation account whose session is supplied by
a root/service-owned credential broker or OS credential store. Broker signing
keys and raw sessions must be unreadable by the evaluation account, outside
workspace/artifact trees, and absent from child environments. An operator may
place a non-secret canary in the disk session area and expose only its path to
the parent harness through `CASEGRAPHEN_EVAL_CREDENTIAL_CANARY_FILE`; that
variable is removed from the child. If the canary reaches stdout, stderr, or a
generated file, output is redacted, the workspace is withheld, and the
scenario fails. This detects artifact leakage; it does not replace OS isolation
or prove prevention of arbitrary network exfiltration.

Every result records the provider executable/version, exact command identity,
model declaration, Skill/input/prompt/output hashes, timestamps, timeout,
stdout/stderr, produced workspace, deterministic evaluator output, and manual
judgments. Provider-emitted token/cost objects are retained without being
upgraded to trusted facts. If cost is not emitted, the summary says
`observable: false`; the release policy requires an explicit waiver rather
than treating missing usage as zero.

When the CLI reports a resolved model identity, it must exactly equal the
requested model ID. An alias that resolves to a different model is retained as
an observation and fails the run; model reproducibility is not inferred from a
request string alone. Providers that emit no model identity remain explicitly
unobservable rather than being treated as a match.

The output `summary.json` includes a content hash over its canonical content.
GitHub's opt-in `Fresh Agent Release Evaluation` workflow uploads the complete
provider directories. Apply `evals/fresh-agent/release-policy.v0.json` during
release review. No run accepts evidence, topology, or runtime output.

After the run, the privileged provider-host broker independently repeats the
safe CLI-session classification and signs the exact summary hash and random run
challenge:

```sh
python3 scripts/fresh-agent-host-attest.py \
  --summary artifacts/fresh-agent/codex/summary.json \
  --provider codex \
  --provider-cli /opt/casegraphen/bin/codex \
  --key-file /run/casegraphen-broker/codex-attestation.key \
  --key-id codex-cli-session-host-v1 \
  --runner-instance-id-hash sha256:<opaque-runner-id-hash> \
  --output /secure-transfer/codex-host-attestation.json
```

The signing key is never copied to the evaluation account, transfer directory,
or an artifact. Repeat on the Claude host with its policy key id.

## Strict matrix aggregation

Provider-lane success is not release success. After both complete ten-scenario
runs are retained, aggregate them against the checked-in baseline and release
threshold:

```sh
python3 scripts/fresh-agent-release.py \
  --provider-run artifacts/fresh-agent/codex \
  --provider-run artifacts/fresh-agent/claude \
  --manual-review artifacts/fresh-agent/manual-review.json \
  --host-attestation codex=/secure-transfer/codex-host-attestation.json \
  --host-attestation claude=/secure-transfer/claude-host-attestation.json \
  --attestation-key codex=/run/release-verifier/codex-attestation.key \
  --attestation-key claude=/run/release-verifier/claude-attestation.key \
  --output-dir artifacts/fresh-agent/release
```

`manual-review.json` has schema
`casegraphen.eval.fresh_agent_manual_review.v0`. Its
`run_content_hashes` object must exactly bind the Codex and Claude
`summary.json` content hashes. It contains one pass/fail judgment per provider
and scenario, with non-empty `reviewer` and `reason`. A review from an earlier
run cannot be replayed against new provider output. If provider cost is not
observable, the same document may contain a `cost_waivers` entry with provider,
reviewer, reason, and a positive `maximum_usd`. The provider run's declared
budget must not exceed that reviewer-authorized limit. The exact
`run_content_hashes` binding prevents replaying the waiver against another run.

The aggregator requires exactly both providers and all ten scenarios. Missing
or duplicate results, provider/version unavailability, runner failures,
timeouts, deterministic evaluator regression, unresolved/failed manual
judgment, unobserved cost, or budget overrun cannot pass. Expected evaluator
kinds are fixed in `release-baseline.v0.json`; thresholds remain fail-closed in
`release-policy.v0.json`.

The aggregator also requires a valid provider-specific HMAC attestation bound
to the exact summary hash, run challenge, authentication class, opaque runner
identity, and brokered credential boundary. A summary's own `cli_session`
fields are only a runner assertion and cannot make a run promotion-eligible.
Attestation keys are verifier inputs and are never retained in the evidence
inventory.

Every retained provider file is copied into a SHA-256 blob store and listed in
the release report. The report itself is named by its content hash. A failed
matrix emits only content-addressed, unreviewed audit/redesign proposals with
`accepted: false`; it never changes an accepted topology.

The GitHub workflows preserve that separation as an explicit evidence
lifecycle:

1. `fresh-agent-release-eval.yml` creates the immutable evaluator and both
   provider artifacts. Its preliminary aggregate intentionally fails at the
   review/attestation seam.
2. A privileged provider broker dispatches
   `fresh-agent-host-attest.yml` once per provider. The broker job uses a
   dedicated label and protected environment, downloads the exact
   commit-named provider artifact, reprobes the non-API CLI session, and emits
   only a content-bound attestation. The HMAC key is not available to the
   evaluation job or retained as evidence.
3. An independent reviewer commits a JSON document under
   `docs/evals/fresh-agent/reviews/` that binds both run hashes and resolves all
   twenty manual judgments. A cost waiver, when needed, is bounded and bound
   to the same run.
4. `fresh-agent-release-finalize.yml` runs in the protected
   `fresh-agent-release-verifier` environment. It downloads the exact provider
   and attestation artifacts by workflow run ID and evaluated commit, uses the
   immutable release aggregator shipped with that evaluation, retains the
   content-addressed final report for 90 days, and fails unless the strict
   aggregate passes.

The repository workflow never manufactures the independent review or broker
authority. Provisioning the dedicated runner labels, protected environments,
runner identity variables, and verifier-only HMAC keys remains an operator
responsibility. Provider API keys are neither required nor accepted.

Required external configuration is explicit:

| Role | Runner/environment | Protected inputs |
|---|---|---|
| Codex evaluation | `casegraphen-codex-cli-session` / `fresh-agent-cli-session-codex` | authenticated Codex CLI session only |
| Claude evaluation | `casegraphen-claude-cli-session` / `fresh-agent-cli-session-claude` | authenticated Claude Code session only |
| Codex broker | `casegraphen-codex-attestation-broker` / `fresh-agent-attestation-codex` | `CASEGRAPHEN_ATTESTATION_KEY`, `CASEGRAPHEN_RUNNER_INSTANCE_ID_HASH` |
| Claude broker | `casegraphen-claude-attestation-broker` / `fresh-agent-attestation-claude` | `CASEGRAPHEN_ATTESTATION_KEY`, `CASEGRAPHEN_RUNNER_INSTANCE_ID_HASH` |
| Final verifier | hosted / `fresh-agent-release-verifier` | `CASEGRAPHEN_CODEX_ATTESTATION_KEY`, `CASEGRAPHEN_CLAUDE_ATTESTATION_KEY` |

Both broker runners also carry the common
`casegraphen-fresh-agent-broker` label. Runner-instance values are opaque
`sha256:` identifiers, not hostnames or account identifiers. Evaluation
environments receive none of the attestation keys; the final verifier receives
no provider session.

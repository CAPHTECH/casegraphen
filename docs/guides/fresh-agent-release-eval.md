# Fresh-agent release evaluation

The checked-in ten-scenario harness is deterministic infrastructure, not
evidence that a real agent followed a Skill. A release evaluation becomes
evidence only when a `summary.json` from a real provider run is retained with
its raw outputs and reviewed manual judgments.

## Declared matrix

The release matrix is Codex and Claude over all ten scenarios in
`evals/fresh-agent/scenarios.v0.json`. The repository ships explicit
`--runner-profile codex` and `--runner-profile claude` adapters. Missing
executables produce `provider_unavailable` and exit 3; they are never replaced
with the fake test runner. Each scenario runs in a new temporary directory that
contains only its task, declared artifacts, and the selected Skill tree.

Run a provider explicitly:

```sh
python3 scripts/fresh-agent-eval.py \
  --runner-profile codex \
  --runner-package-identity '@openai/codex@0.146.0' \
  --expected-runner-version 0.146.0 \
  --model <release-model-id> \
  --budget-usd 25 \
  --timeout 900 \
  --casegraphen-bin target/release/casegraphen \
  --output-dir artifacts/fresh-agent/codex
```

Use `--runner-profile claude` with the pinned Claude package identity and
version for the Claude lane. A lane is unavailable (exit 3) when its credential
is absent; this is never reported as a passing or simulated run. The workflow
places only `OPENAI_API_KEY` in the Codex execution step and only
`ANTHROPIC_API_KEY` in the Claude execution step. It has no job-level secrets,
is manual-only, and has read-only repository permission, so fork and untrusted
pull-request events cannot obtain provider credentials.

The harness removes every secret-like environment entry before starting a real
provider, then restores only the credential selected by that provider profile.
It never serializes environment variables, redacts exact configured secret
values from stdout/stderr and evaluator details, and scans generated workspace
bytes for those exact values before retention. If a configured secret value is
found, the workspace is withheld and the scenario fails. This is a retention
boundary, not a claim to detect encoded or transformed disclosures. The
policy-declared package identity, expected version, observed version, and
command hash remain in `summary.json` for reproducibility.

Every result records the provider executable/version, exact command identity,
model declaration, Skill/input/prompt/output hashes, timestamps, timeout,
stdout/stderr, produced workspace, deterministic evaluator output, and manual
judgments. Provider-emitted token/cost objects are retained without being
upgraded to trusted facts. If cost is not emitted, the summary says
`observable: false`; the release policy requires an explicit waiver rather
than treating missing usage as zero.

The output `summary.json` includes a content hash over its canonical content.
GitHub's opt-in `Fresh Agent Release Evaluation` workflow uploads the complete
provider directories. Apply `evals/fresh-agent/release-policy.v0.json` during
release review. No run accepts evidence, topology, or runtime output.

## Strict matrix aggregation

Provider-lane success is not release success. After both complete ten-scenario
runs are retained, aggregate them against the checked-in baseline and release
threshold:

```sh
python3 scripts/fresh-agent-release.py \
  --provider-run artifacts/fresh-agent/codex \
  --provider-run artifacts/fresh-agent/claude \
  --manual-review artifacts/fresh-agent/manual-review.json \
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

Every retained provider file is copied into a SHA-256 blob store and listed in
the release report. The report itself is named by its content hash. A failed
matrix emits only content-addressed, unreviewed audit/redesign proposals with
`accepted: false`; it never changes an accepted topology.

The GitHub workflow runs aggregation without fabricating manual review.
Consequently the aggregate job remains blocked until an independent reviewer
downloads the evidence, creates a run-bound review document, and reruns the
command above.

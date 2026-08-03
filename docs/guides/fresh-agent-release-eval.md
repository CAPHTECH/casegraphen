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
  --model <release-model-id> \
  --budget-usd 25 \
  --timeout 900 \
  --casegraphen-bin target/release/casegraphen \
  --output-dir artifacts/fresh-agent/codex
```

Use `--runner-profile claude` for the Claude lane. Credentials remain in the
process environment. The harness never serializes environment variables and
redacts exact values of token/secret/password/API-key variables from captured
stdout and stderr. Review raw output for provider-side disclosure before
publication.

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

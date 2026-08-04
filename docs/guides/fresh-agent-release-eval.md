# Fresh-agent release evaluation

The checked-in ten-scenario harness is deterministic infrastructure, not
evidence that a real agent followed a Skill. A release evaluation becomes
evidence only when a `summary.json` from a real provider run is retained with
its raw outputs, an evaluation-runner host/session proof, a provider-broker
countersignature, and independently signed manual judgments. The checked-in
workflows implement this authority boundary, but their presence is not evidence
that the required protected environments, runners, keys, or approvals have
been provisioned or exercised.

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

After each provider artifact is uploaded, an externally provisioned attestor
on the evaluation host creates an Ed25519 `evaluation_host_session_proof`. It
observes the actual provider-bearing runner rather than asking the later broker
to reproduce a CLI probe on a different host. The proof binds the evaluation
run ID and attempt, workflow name and path, head ref and SHA, provider artifact
ID/name/digest, summary content hash and random challenge, opaque runner
identity, authentication classification, CLI-session result, and the declared
credential-isolation boundary. Its private key and implementation are external
host controls: neither is supplied by the repository bundle or broker job.

The provider-specific broker then independently observes the GitHub run and
artifact, verifies the evaluation-host proof with a protected Ed25519 public
key and pinned SPKI fingerprint, and signs that exact proof/provenance join with
its own provider-specific Ed25519 private key:

```sh
python3 scripts/fresh-agent-host-attest.py \
  --summary artifacts/fresh-agent/codex/summary.json \
  --provider codex \
  --private-key-file /run/casegraphen-broker/codex-attestation.pem \
  --key-id codex-cli-session-host-v1 \
  --evaluation-host-proof /secure-transfer/codex-evaluation-host-proof.json \
  --evaluation-host-public-key /run/casegraphen-broker/codex-evaluation-host-public.pem \
  --evaluation-host-key-id <protected-evaluation-host-key-id> \
  --evaluation-host-public-key-spki-sha256 sha256:<protected-spki-hash> \
  --provenance-file /secure-transfer/codex-run-provenance.json \
  --output /secure-transfer/codex-host-attestation.json
```

The broker obtains the provenance document by querying GitHub for the exact
repository, evaluation workflow path, run ID, run attempt, main-branch head
SHA, `workflow_dispatch` event, successful conclusion, and uniquely named
provider artifact. The artifact ID, name, GitHub digest, and independently
downloaded archive digest are fixed in that document. GitHub API requests
refuse redirects. Artifact downloads send the GitHub token only to
`api.github.com`, follow a bounded allowlist to GitHub's Azure blob host without
that token, and reject a redirect back to the authenticated origin. The broker
signing key is never copied to the evaluation account, transfer directory,
finalizer, or an artifact. Repeat on the Claude broker with its
provider-specific host-proof key and broker key.

## Strict matrix aggregation

Provider-lane success is not release success. After both complete ten-scenario
runs are retained, aggregate them against the checked-in baseline and release
threshold:

```sh
python3 scripts/fresh-agent-release.py \
  --provider-run artifacts/fresh-agent/codex \
  --provider-run artifacts/fresh-agent/claude \
  --manual-review artifacts/fresh-agent/signed-manual-review.json \
  --manual-review-public-key /run/release-verifier/reviewer-public.pem \
  --expected-reviewer-identity <protected-reviewer-identity> \
  --expected-reviewer-key-id <protected-reviewer-key-id> \
  --host-attestation codex=/secure-transfer/codex-host-attestation.json \
  --host-attestation claude=/secure-transfer/claude-host-attestation.json \
  --attestation-public-key codex=/run/release-verifier/codex-public.pem \
  --attestation-public-key claude=/run/release-verifier/claude-public.pem \
  --expected-provenance codex=/secure-transfer/codex-run-provenance.json \
  --expected-provenance claude=/secure-transfer/claude-run-provenance.json \
  --output-dir artifacts/fresh-agent/release
```

The human first authors an unsigned review under
`docs/evals/fresh-agent/reviews/`. Its `run_content_hashes` object must exactly
bind the Codex and Claude `summary.json` content hashes. It contains one
pass/fail judgment per provider and scenario with a non-empty `reason`. If
provider cost is not observable, the same document may contain a
`cost_waivers` entry with provider, reason, and a positive `maximum_usd`; the
run's declared budget must not exceed that limit. The unsigned document is not
release authority merely because it is committed.

`fresh-agent-manual-review-sign.yml` checks out the exact review-source commit
and an exact trusted-verifier commit, constrains the input to this review
directory, and runs in the protected `fresh-agent-manual-review-signer`
environment. Its Ed25519 signer adds the schema
`casegraphen.eval.fresh_agent_manual_review.v1`, the protected reviewer identity
and key ID, `signature_algorithm: ed25519`, and a signature over the canonical
review payload. Before signing, it observes both provider artifacts through the
GitHub API and fixes each exact workflow/run/attempt/head SHA and artifact
ID/name/digest in `expected_provider_provenance`. The input must be a regular,
non-symlink JSON file whose resolved path remains below the allowed review
directory. Duplicate JSON members and non-finite numbers are rejected. The
private key is temporary signer input and is not published. A review from an
earlier run or another artifact cannot be replayed against new provider output.

The aggregator requires exactly both providers and all ten scenarios. Missing
or duplicate results, provider/version unavailability, runner failures,
timeouts, deterministic evaluator regression, unresolved/failed manual
judgment, unobserved cost, or budget overrun cannot pass. Expected evaluator
kinds are fixed in `release-baseline.v0.json`; thresholds remain fail-closed in
`release-policy.v0.json`.

The aggregator also requires a valid provider-specific Ed25519 attestation
bound to the exact summary hash, run challenge, authentication class, opaque
runner identity, externally attested credential boundary, and the exact
observed workflow and artifact provenance. A summary's own `cli_session` fields
are only a runner assertion and cannot make a run promotion-eligible. The
finalizer receives only the public keys. It parses each key as an Ed25519 SPKI
key and checks its SHA-256 SPKI fingerprint against protected configuration
before verification.

Every retained provider file is copied into a SHA-256 blob store and listed in
the release report. The report itself is named by its content hash. A failed
matrix emits only content-addressed, unreviewed audit/redesign proposals with
`accepted: false`; it never changes an accepted topology.

The GitHub workflows preserve that separation as an explicit evidence
lifecycle. Dispatch them in this order and retain every run ID and run attempt:

1. `fresh-agent-release-eval.yml` creates the immutable evaluator and both
   provider artifacts and separate evaluation-host proofs. Its preliminary
   report remains `promotion_eligible: false` at the review/attestation seam,
   while the workflow itself succeeds
   when both provider lanes complete; a provider/evaluator failure still fails
   the workflow. The proof is produced by the external attestor on the actual
   evaluation runner after upload metadata is known. Record the run ID, run
   attempt, and evaluated commit SHA.
2. Dispatch `fresh-agent-host-attest.yml` once for Codex and once for Claude,
   passing the exact evaluation run ID, attempt, and evaluated commit. Each
   provider broker uses dedicated labels and a protected environment, checks
   out `CASEGRAPHEN_TRUSTED_VERIFIER_SHA`, verifies that exact checkout, obtains
   the exact provider artifact and evaluation-host proof by GitHub API, verifies
   the provider-specific host public key/SPKI and proof, and emits a second
   Ed25519-signed attestation. It does not claim authority by probing a broker
   machine's own CLI session. Record each attestation run ID, attempt, and head
   SHA.
3. An independent human authors an unsigned JSON document under
   `docs/evals/fresh-agent/reviews/` that binds both run hashes and resolves all
   twenty manual judgments. Commit it and record the exact review-source SHA.
4. Dispatch `fresh-agent-manual-review-sign.yml` with that source SHA and path.
   The protected signer checks out the exact trusted verifier and review
   source, then publishes only the signed review artifact. Record the signing
   run ID, attempt, and head SHA.
5. Dispatch `fresh-agent-release-finalize.yml` with the evaluation,
   attestation, signing, and source coordinates above. The protected
   `fresh-agent-release-verifier` environment downloads the exact provider
   artifacts, attestations, and signed review by run ID, run attempt, workflow
   path, and head SHA. It checks out the exact trusted-verifier SHA rather than
   accepting verifier code from the evaluated bundle, validates all public-key
   fingerprints and provenance, retains the content-addressed final report for
   90 days, and fails unless the strict aggregate passes. The durable report
   includes the verifier public PEM files, their key provenance, release
   policy, evaluator baseline, scenario manifest, both evaluation-host proofs
   and host public keys, broker attestations, workflow provenance, and trusted
   verifier source inventory.
6. Only after a successful finalization, its publisher job packages the report
   deterministically and creates a GitHub Release tag
   `fresh-agent-evidence-<package-sha256>` with asset
   `sha256-<package-sha256>.tar.gz`. A retry accepts only the same tag, target,
   and exact zero-or-one asset inventory; it creates or resumes the missing
   publication without overwrite, then redownloads the asset to verify its
   digest. Any different, duplicate, or mismatched asset fails closed.

`CASEGRAPHEN_TRUSTED_VERIFIER_SHA` is a protected environment variable
containing an exact 40-character commit SHA. Every session- or authority-bearing
job also requires the dispatched privileged workflow YAML itself to have that
exact `github.sha`; it cannot run privileged code merely by checking out a
trusted helper later. Broker, review-signer, finalizer, and publisher jobs then
check out and assert the same SHA before invoking authority code.
The repository workflow never manufactures the independent review or broker
authority. Provisioning the dedicated runner labels, protected environments,
required reviewers, identities, keys, fingerprints, and variables remains an
operator responsibility. Provider API keys are neither required nor accepted.

Required external configuration is explicit:

| Role | Runner/environment | Protected inputs |
|---|---|---|
| Codex evaluation | common label `casegraphen-fresh-agent` plus `casegraphen-codex-cli-session` / `fresh-agent-cli-session-codex` | authenticated Codex CLI session; vars `CASEGRAPHEN_EVALUATION_HOST_ATTESTOR`, `CASEGRAPHEN_EVALUATION_HOST_KEY_ID`; externally held host-proof private key |
| Claude evaluation | common label `casegraphen-fresh-agent` plus `casegraphen-claude-cli-session` / `fresh-agent-cli-session-claude` | authenticated Claude Code session; vars `CASEGRAPHEN_EVALUATION_HOST_ATTESTOR`, `CASEGRAPHEN_EVALUATION_HOST_KEY_ID`; externally held host-proof private key |
| Codex broker | common label `casegraphen-fresh-agent-broker` plus `casegraphen-codex-attestation-broker` / `fresh-agent-attestation-codex` | secret `CASEGRAPHEN_ATTESTATION_PRIVATE_KEY`; vars `CASEGRAPHEN_EVALUATION_HOST_PUBLIC_KEY`, `CASEGRAPHEN_EVALUATION_HOST_KEY_ID`, `CASEGRAPHEN_EVALUATION_HOST_PUBLIC_KEY_SPKI_SHA256`, `CASEGRAPHEN_TRUSTED_VERIFIER_SHA` |
| Claude broker | common label `casegraphen-fresh-agent-broker` plus `casegraphen-claude-attestation-broker` / `fresh-agent-attestation-claude` | secret `CASEGRAPHEN_ATTESTATION_PRIVATE_KEY`; vars `CASEGRAPHEN_EVALUATION_HOST_PUBLIC_KEY`, `CASEGRAPHEN_EVALUATION_HOST_KEY_ID`, `CASEGRAPHEN_EVALUATION_HOST_PUBLIC_KEY_SPKI_SHA256`, `CASEGRAPHEN_TRUSTED_VERIFIER_SHA` |
| Manual review signer | hosted / `fresh-agent-manual-review-signer` | secret `CASEGRAPHEN_REVIEWER_PRIVATE_KEY`; vars `CASEGRAPHEN_REVIEWER_IDENTITY`, `CASEGRAPHEN_REVIEWER_KEY_ID`, `CASEGRAPHEN_TRUSTED_VERIFIER_SHA` |
| Final verifier | hosted / `fresh-agent-release-verifier` | vars `CASEGRAPHEN_CODEX_ATTESTATION_PUBLIC_KEY`, `CASEGRAPHEN_CODEX_ATTESTATION_PUBLIC_KEY_SPKI_SHA256`, `CASEGRAPHEN_CLAUDE_ATTESTATION_PUBLIC_KEY`, `CASEGRAPHEN_CLAUDE_ATTESTATION_PUBLIC_KEY_SPKI_SHA256`, `CASEGRAPHEN_REVIEWER_PUBLIC_KEY`, `CASEGRAPHEN_REVIEWER_PUBLIC_KEY_SPKI_SHA256`, `CASEGRAPHEN_REVIEWER_IDENTITY`, `CASEGRAPHEN_REVIEWER_KEY_ID`, `CASEGRAPHEN_TRUSTED_VERIFIER_SHA` |
| Durable publisher | hosted / `fresh-agent-evidence-publisher` | var `CASEGRAPHEN_TRUSTED_VERIFIER_SHA`; job-scoped GitHub token with `contents: write` |

Runner-instance values are opaque `sha256:` identifiers emitted inside the
external host proof, not caller-supplied workflow variables, hostnames, or
account identifiers. Evaluation environments receive no broker-attestation or
review private keys; the external host attestor must keep its own private key
outside repository-controlled process state. The final verifier receives no
provider session or private key. Public keys are not secrets, but their
protected values and pinned SPKI fingerprints are part of the verifier's trust
configuration.

Actions artifacts in this lifecycle are retained for 90 days and therefore
form a bounded review window, not durable archival storage. The successful
publisher's content-addressed GitHub Release asset extends retention and is
create-new in this workflow, but GitHub Releases remain mutable or deletable by
authorized repository operators. This mechanism is explicitly not WORM
storage. Stable promotion still requires actual workflow evidence satisfying
these checks; checked-in workflow definitions alone do not establish it.

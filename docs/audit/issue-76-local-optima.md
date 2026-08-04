# Issue #76 implementation local-optima audit

## Scope and evidence

- `B`: repository scripts and workflows, GitHub configuration, provider and
  broker OS accounts, reviewer authority, durable publication, and later
  release audit.
- `M`: not only lane completeness, but non-forgeable authority, exact source
  provenance, independent review, retrievability, and fail-closed acceptance.
- `N`: evaluation scripts, workflows, signature scheme, runner/environment
  configuration, review signer, retention backend, tests, and documentation.
  Stable ledger decision rules remain unchanged.
- `T`: provider execution through product-lifetime re-audit, including key
  rotation, workflow reruns, crashes between publish/verify, and incident
  investigation after Actions artifacts expire.

Evidence planes:

1. Strict matrix tests execute the exact two-provider by ten-scenario baseline,
   missing/unavailable/timeout refusals, content-addressed retention, and a
   run-bound cost waiver with a reviewer-authorized limit.
2. Runtime pilot tests execute four distinct local runtime families. SQLite
   durable-queue and asyncio event-stream families additionally exercise
   generic JSONL artifact ingest and exact resource reconciliation.
3. A Python standard-library MCP client independently completes initialize,
   propose, lint, compile, attach, and reconcile, then stops at the review
   seam with `accepted: false`. Its deterministic report is retained in
   `docs/pilots/issue-76/independent-mcp-client-report.json` rather than being
   deleted with the test workspace.
4. `docs/pilots/issue-76/retained-evidence.manifest.json` binds the generated
   JSONL streams and reports to SHA-256 digests and byte lengths.
5. A current authenticated-CLI local rerun completed Codex and Claude 10/10
   with no deterministic failure, timeout, or credential-retention finding.
   The unsigned-authority aggregate is
   `sha256:3fba65c1e8409da667ce31b5965dea400c4a9be28545ae09952ba1b0b00906ed`
   and fails on both missing host attestations, missing signed review,
   unresolved judgments, and unobserved Codex cost. This is negative evidence
   for promotion, not a substitute for the protected workflow lifecycle.
6. Current-HEAD remote run
   [`30956779910`](https://github.com/CAPHTECH/casegraphen/actions/runs/30956779910)
   built and published the immutable evaluator, then left both provider jobs
   queued with their exact self-hosted labels and `runner_id: null`. The run
   was cancelled rather than left queued indefinitely. Its aggregate still
   emitted the content-addressed report
   `sha256:622e17fbccc8de67b5a44f1d66f0b3ae3f61a14e8153f17588563f1a4318ba8c`
   with `accepted: false`, `promotion_eligible: false`, and findings
   `missing_provider:codex`, `missing_provider:claude`, and
   `manual_review_missing`. This is direct fail-closed operational evidence,
   not provider execution evidence.

## Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Let a broker's own CLI probe stand for the evaluation runner | Easy post-run check | A different host/session is attested | Broker host to evaluated host | 14 | C3 | `boundary inversion`, resolved by an evaluation-runner external Ed25519 proof |
| 2 | Execute evaluated verifier code or privileged YAML outside the trusted SHA | Exact version alignment | Every evaluated/ref-selected commit enters the privileged TCB | Evaluated workflow to signing domain | 13 | C3 | `mixed`, resolved: privileged jobs require exact `github.sha`, then protected exact trusted source consumes evaluated data |
| 3 | Give the finalizer both HMAC secrets | Simple sign/verify fixture | Verifier can forge both broker identities | Workflow role to cryptographic authority | 12 | C2 | `externalization`, resolved with Ed25519 public verification |
| 4 | Accept caller-declared reviewer identity, provenance, and waiver | Easy human-review join | Reviewer independence, source identity, and financial authority are unverifiable | JSON content to organizational authority | 12 | C3 | `organizational`, resolved in contract with signed API-observed provenance; provisioning remains absent |
| 5 | Join workflows using unchecked run IDs and artifact names | Simple dispatch | Operator must prevent wrong workflow/ref/attempt/artifact replay | Workflow orchestration to provenance authority | 10 | C2 | `mixed`, resolved with API-observed signed provenance |
| 6 | Forward GitHub authorization through artifact redirects | Convenient default HTTP handling | Repository token crosses into blob-storage requests | API identity to storage transport | 10 | C3 | `boundary inversion`, resolved by explicit redirect/host/token policy |
| 7 | Call 90-day Actions artifacts durable release evidence | Low storage/operations cost | Bytes disappear before product-lifetime re-audit | Review window to release lifetime | 9 | C2 | `time-delayed`, resolved with hash-named Release asset; GitHub is not WORM |
| 8 | Repeat two small broker jobs instead of parameterizing authority routing | Visible duplication | Minor maintenance duplication | Workflow source only | 2 | C2 | `not-local-optimum`; intentional authority separation |

The severity column uses `E + A + F + K + T` (0–15). It describes each
pre-fix candidate, while the verdict records the current contract. At the time
of remote run `30956779910`, the repository API reported no repository-scoped
self-hosted runners and neither provider job received a runner. Subsequent
Issue #89 work provisioned the two CLI-session Environment names, a separate
runtime-durability publisher Environment, and an exact trusted-SHA variable;
those controls do not supply the missing provider or broker runners, external
host attestors, provider/broker key material, signer/finalizer Environments, or
main protection required by this issue. Organization-scoped runners,
variables, and secrets could not be inventoried because the active token lacks
`admin:org`. These are material missing operational evidence, not facts that
repository conformance can manufacture.

## Observations

- [Evidence] The aggregator requires exactly Codex and Claude, all ten scenario
  IDs, pinned runner identity/version, the exact scenario-manifest hash, and
  the baseline evaluator kinds.
- [Evidence] Pinned profiles classify only known non-API CLI sessions; the
  workflow assigns provider-specific hosts and injects no provider key. A
  summary remains a caller assertion until an external attestor on the actual
  evaluation runner signs its run/attempt/workflow/head/artifact provenance,
  content hash, challenge, runner identity, session class, and credential
  isolation. A separate provider broker verifies the host public key/SPKI and
  proof before countersigning; its own CLI state is irrelevant.
- [Evidence] Provider execution does not optimize completion by bypassing tool
  permissions: Codex is workspace-write/ephemeral with user config ignored,
  while Claude is limited to project Read/Write/Edit without ambient MCP.
- [Evidence] A provider-reported resolved model must equal the requested model;
  an accepted alias that resolves elsewhere fails instead of masquerading as
  exact model evidence.
- [Evidence] Evidence review, stale-revision handling, and failure/halt recovery
  now have deterministic output contracts. Their manual judgments were narrowed
  to qualitative rationale so reviewers do not re-evaluate the exact fields.
- [Evidence] Checkout, dependency installation, and evaluator build occur in an
  uncredentialed hosted prepare job. Authenticated provider runners consume
  only the prepared artifact, use an absolute evaluator path, and bind workflow
  inputs through quoted environment-derived argv.
- [Evidence] Embedded summary results must equal retained per-scenario
  `result.json`; symlinked evidence is refused rather than followed.
- [Evidence] Manual judgments and cost waivers bind to both provider run content
  hashes, exact provider run/artifact provenance, and an Ed25519-verified
  reviewer identity/key ID. The signer confines a regular non-symlink input to
  the review directory and rejects duplicate-key or non-finite JSON. Each
  waiver has a reason and positive `maximum_usd`, and cannot authorize a
  provider-declared budget above that limit. Unsigned local judgments supply no
  authority.
- [Evidence] Authority-bearing workflows require their own `github.sha` to
  equal protected `CASEGRAPHEN_TRUSTED_VERIFIER_SHA`, then check out and assert
  that same exact source before running verifier code. Pinning only a helper
  checkout would not constrain untrusted privileged workflow YAML.
- [Evidence] GitHub API calls refuse redirects. Artifact retrieval allows a
  bounded transition from `api.github.com` to the specific GitHub Azure blob
  host without the token, and refuses return to the authenticated API origin.
- [Evidence] Release and pilot evidence are content-addressed; release failures
  emit only unreviewed audit/redesign proposals with `accepted: false`.
- [Evidence] Complete runtime reconciliation and the independent MCP client
  both stop at `needs_review`.
- [Inference] The shared generic JSONL/resource seam is exercised through four
  materially different transport/storage families. Issue #85 separately adds
  bounded remote, binary, scale, retry, crash/resume, and allocator-journal
  evidence; neither bounded suite proves a production fleet indefinitely.

## Local rationality and compensation halo

Initially, trusting the aggregate summary was locally attractive because the
provider runner already generated it. That moved completeness verification to
human review: retained `result.json` files, provider identity, and manifest
binding could disagree while the aggregate appeared complete. Classification:
`externalization`, severity E2/A1/F2/K2/T1 = 8, confidence C3. The fix compares
retained results exactly and validates provider pins and manifest identity.

Walking retained evidence recursively was convenient, but following symlinks
would let a run inventory bytes outside its evidence directory. Classification:
`boundary inversion`, severity E2/A0/F2/K1/T1 = 6, confidence C3. The retention
step now refuses symlinks.

The baseline originally carried a manifest field without validating it, making
future scenario drift a downstream discovery. Classification: `temporal`,
severity E1/A1/F2/K1/T2 = 7, confidence C3. Baseline validation now checks the
manifest schema and its exact scenario/evaluator inventory.

Finally, treating “cost waived” as a provider boolean would optimize matrix
completion while exporting financial risk to the reviewer. Classification:
`externalization`, severity E2/A1/F2/K2/T2 = 9, confidence C3. The waiver is now
part of an independently signed review, bound to exact run hashes, reasoned,
and capped; the provider's declared maximum must fit inside that cap.

After the authentication contract was clarified, checking only package and
version would have let an API-authenticated or unauthenticated synthetic summary
look like a valid provider run. This was locally attractive because execution
preflight appeared to own authentication, while the aggregate evidence consumer
paid the provenance gap. Classification: `boundary inversion`, severity
E3/A1/F2/K3/T2 = 11, confidence C3. Aggregation now independently requires the
policy's allowed non-API session class, exact policy pin, proof that auth probe
output was not retained, and a valid evaluation-host proof plus broker
countersignature. Missing or substituted signatures and a completed-looking
caller-only assertion are release failures.

The first release workflow optimized job count by building in the authenticated
provider job and directly interpolating dispatch inputs into shell text. That
made the session-bearing runner absorb repository/dependency execution and
shell-parsing risk. Classification: `boundary inversion`, severity
E3/A2/F2/K3/T2 = 12, confidence C3. A hosted prepare job now creates the
short-lived bundle; SHA-pinned artifact actions are the only external action
code on the provider lane, provider-label pairing is checked structurally, and
unsafe input/path mutations have negative conformance tests.
Provider execution is additionally restricted to `refs/heads/main` and names a
provider-specific GitHub Environment; the hosted prepare job may run elsewhere,
but a non-main ref or a ref whose SHA differs from the protected trusted SHA
cannot reach the session-bearing job. Broker, reviewer, finalizer, and publisher
jobs enforce the same privileged-workflow SHA before using authority. Actual
Environment reviewer/branch protection remains external configuration and is
not inferred from the workflow string.

The first fail-closed aggregate also stopped after uploading provider output
and told an operator to download artifacts, obtain two attestations, author the
review, reconstruct six security-sensitive arguments, rerun aggregation, and
retain the result manually. That preserved the trust seam but optimized the
first workflow by externalizing the complete evidence lifecycle.
Classification: `externalization`, severity E2/A2/F2/K2/T2 = 10, confidence
C3. The lifecycle is now explicit and conformance-gated: provider-specific
broker jobs consume exact run artifacts without running evaluation, a protected
reviewer signs the complete review, and a protected hosted finalizer receives
only public keys. Intermediate evidence is retained for the 90-day review
window, while a passing deterministic package is published under a
content-addressed GitHub Release asset and re-downloaded for hash verification.
GitHub Release deletion remains administratively possible and is not described
as WORM storage.

The first encoded broker/finalizer lifecycle downloaded and executed
`fresh-agent-host-attest.py` and `fresh-agent-release.py` from the evaluated
bundle. Exact version alignment was locally convenient, but it admitted every
evaluated commit into the privileged signing/verifying TCB. Classification:
`mixed`, severity E3/A2/F3/K3/T2 = 13, confidence C2. Broker, reviewer, and
finalizer now checkout an exact protected `CASEGRAPHEN_TRUSTED_VERIFIER_SHA`;
evaluated artifacts are parsed only as data. Trusted source SHA and script
hashes are retained in the durable package. A separately governed verifier
repository would further narrow the TCB; same-repository protected SHA remains
the documented experimental compromise.

That first lifecycle also used HMAC and supplied both symmetric secrets to the
finalizer. The role named “verifier” therefore possessed authority to forge
both broker signatures. Classification: `externalization`, severity
E3/A2/F2/K3/T2 = 12, confidence C2. Provider-specific Ed25519 private keys now
remain broker-only. The finalizer consumes public PEM values, verifies their
protected SPKI fingerprints, and cannot sign an attestation with those values.

The manual-review parser previously checked only non-empty reviewer/reason
strings and run hashes. This made content binding strong but reviewer identity
caller-constructible. Classification: `organizational`, severity
E3/A2/F2/K3/T2 = 12, confidence C3. A protected reviewer workflow now signs the
entire review document, including both run hashes, every judgment, and every
cost waiver. The finalizer maps the signature to protected reviewer identity,
key ID, and public-key fingerprint. Local unsigned reviews remain diagnostics
and deliberately produce unresolved manual judgments.

Commit-shaped artifact names were identifiers, not provenance. Syntax-valid
run IDs could refer to another workflow, attempt, branch, or same-named
artifact. Classification: `mixed`, severity E2/A2/F2/K2/T2 = 10, confidence
C2. Trusted code now resolves GitHub run and artifact metadata, requires the
allowlisted workflow path, main branch, exact head SHA, successful attempt,
artifact ID/name/digest, verifies downloaded ZIP bytes, and binds the observed
document into each broker signature and final aggregate.

The first broker design repeated the provider CLI auth probe on the broker and
called that a provider-host attestation. That was operationally simple but
proved the broker's session, not the account/runner that produced the evaluated
bytes. Classification: `boundary inversion`, severity E3/A2/F3/K3/T3 = 14,
confidence C3. The evaluation runner now obtains a provider-specific external
Ed25519 host proof after the artifact ID/digest is known. That proof binds the
exact run, attempt, workflow, head, artifact, summary hash/challenge, runner
identity, auth class, and credential isolation. The broker only verifies that
proof against a protected host public key/SPKI and joins it to independently
observed GitHub provenance before countersigning.

Default redirect handling would have allowed the authenticated GitHub artifact
request to carry its bearer token across origin. Classification: `boundary
inversion`, severity E3/A1/F3/K2/T1 = 10, confidence C3. Trusted provenance code
now refuses API redirects, manually follows at most three artifact redirects,
allowlists only GitHub's Azure blob host after leaving `api.github.com`, strips
authorization there, and rejects return to the authenticated origin.

Signing reviewer-authored JSON without constraining path or parse semantics
would make a valid signature compatible with symlink substitution, duplicate
members, or non-finite numeric values whose interpretation differs by parser.
Classification: `mixed`, severity E2/A1/F2/K2/T1 = 8, confidence C3. The signer
requires a resolved regular non-symlink file below the review root, uses strict
duplicate/non-finite rejection, and includes independently API-observed
provider provenance and artifact digests in the signed payload.

An early real Claude run requested a model alias that the CLI resolved to a
different canonical model. Treating the request string as the executed model
would optimize a green matrix at the cost of reproducibility. Classification:
`temporal`, severity E2/A1/F2/K2/T2 = 9, confidence C3. The harness now records
reported model identities and fails an observable mismatch; the final run must
request the reported canonical ID.

One real Codex design serialized the two requested writers but introduced
analysis nodes whose live-file reads could overlap the first write. Optimizing
the visible writer/writer edge left a hidden read/write conflict.
Classification: `boundary inversion`, severity E2/A1/F2/K2/T1 = 8, confidence
C3. The design Skill now requires ordering every conflicting resource pair and
recommends a read barrier or immutable snapshot; the canonical linter remains
the decision owner.

Promoting the three safety conditions from manual-only review to deterministic
fields is locally attractive because it removes reviewer variance. Keeping the
same exact condition in the manual checklist would merely shift duplicated work
to release reviewers. Classification: `externalization`, severity
E1/A1/F1/K2/T1 = 6, confidence C2. Deterministic assertions now own the exact
action/retry fields; manual review is limited to non-fabrication and rationale.

## Counterfactuals

- A — Keep per-provider summaries and rely on a release reviewer to find
  omissions: small implementation, large compensation halo. Rejected.
- B — Add only matrix counting: catches missing rows but not substituted files,
  stale reviews, cost authority, or evidence provenance. Rejected.
- C — Validate exact retained evidence, bind independent decisions to run
  hashes and limits, require asymmetric broker-signed run/host CLI-session
  provenance and signed reviewer identity, retain content-addressed reports,
  and keep all outputs unaccepted until review. Adopted.
- D — Keep broker/reviewer authority external but encode artifact transfer,
  API-observed exact-run joining, public-key handling, final aggregation,
  release publication, and fail-closed disposition as separate workflows.
  Adopted; this removes operator argument reconstruction without collapsing
  authority roles.

## Widened-boundary comparison and migration valley

| Evaluation boundary | Narrow implementation benefit | Cost after widening the boundary | Adopted compensation | Current advantage |
|---|---|---|---|---|
| Provider process | One CLI invocation can emit a complete-looking summary | The process cannot attest its own account isolation or hidden signing key | External evaluation-host proof over exact run, artifact, challenge, and isolation | Evaluation-runner proof plus separate broker countersignature |
| Provider workflow | Building and evaluating in one job minimizes orchestration | Repository code executes beside an authenticated CLI session | Uncredentialed prepare job and immutable evaluator artifact | Split prepare/evaluate |
| Broker/finalizer TCB | Executing evaluated scripts prevents version drift | Evaluated code or workflow YAML receives signing or verification material | Privileged `github.sha` and protected exact verifier source must match | Trusted-source workflow and execution |
| Cryptographic role | HMAC is simple to implement | Finalizer can forge broker output | Broker-private Ed25519, verifier-public keys | Asymmetric authority |
| Release workflow | Preliminary aggregation is easy to implement | Operators manually join security-sensitive IDs and may join the wrong runs | API-observed run/attempt/workflow/artifact provenance | Encoded lifecycle |
| Artifact transport | Automatic redirects simplify download | GitHub bearer token may cross origin | Refuse API redirects; allowlisted tokenless blob hop only | Explicit credential boundary |
| Review input | Sign an arbitrary JSON path supplied at dispatch | Path and JSON ambiguity weaken the signed meaning | Root confinement, strict JSON, signed provider provenance | Canonical review authority |
| Release operation | Dynamic provider routing removes duplicated YAML | A label/environment/key mix-up becomes less visible at the authority boundary | Two fixed broker jobs with conformance checks | Intentional repetition |
| Evidence lifetime | 90-day Actions artifacts are cheap | Product-lifetime re-audit loses source bytes | Hash-named Release asset plus redownload verification | Durable, not WORM |
| Stable-promotion lifecycle | Local 20/20 is fast feedback | Local success cannot establish audited host/session/reviewer provenance | Promotion remains false without all signatures | Fail-closed local evidence |

The migration carries four authority workflows, two provider artifacts, two
evaluation-host proof artifacts, two broker artifacts, one signed reviewer
document, a final aggregate, and a durable package.
That is more artifact coordination than the original single workflow. The
extra coordination is accepted because each artifact corresponds to a
distinct authority; collapsing them would recreate the provenance gap. The
rollback path is to retain local non-promotion evaluation only, not to accept
an unsigned summary as release evidence.

## False positives and residual risks

- [Evidence] The Codex and Claude broker jobs intentionally repeat a small
  sequence instead of using a dynamic runner/environment expression. Separate
  fixed labels, key IDs, protected environments, and artifact names make a
  provider/key routing swap visible to conformance tests; this is authority
  separation, not accidental decision-rule duplication.
- [Evidence] Similar request/response logic in the runtime pilot and independent
  MCP client is intentional implementation independence, not accidental rule
  duplication; neither owns CaseGraphen decision semantics.
- [Evidence] The aggregate workflow not passing without a signed external
  manual review is the intended trust boundary, not a CI availability defect.
- [Hypothesis] Real authenticated Codex and Claude CLI sessions may reveal
  provider-specific output, session-expiry, or cost-telemetry differences. No
  API key was used. An interrupted CLI-session evaluation is not retained or
  treated as release evidence; promotion still requires its external host proof
  and provider-broker countersignature.
- [Hypothesis] Model comparison applies only when a recognized provider envelope
  emits a non-empty top-level model observation. `observable: false` is retained
  as absence, not evidence that the requested model executed; provider-specific
  nested envelope formats still need real-run calibration.
- [Hypothesis] Ed25519 verification proves possession of the configured broker
  and evaluation-host private keys and signed run binding, not that deployed OS
  accounts, attestor implementation, key ACLs, Environment approval, or
  credential brokers actually prevent agent reads. That remains externally
  auditable host provisioning and must not be inferred from repository tests.
- [Hypothesis] The trusted verifier SHA is protected only when the declared
  GitHub Environment and repository controls actually exist. A separately
  governed reusable-workflow repository would reduce the same-repository TCB,
  but is not provisioned here.
- [Evidence] A GitHub Release asset outlives the Actions review window and is
  content-addressed, but repository administrators can delete it. The workflow
  does not claim object-lock or transparency-log guarantees. The package keeps
  verifier public PEM files/key provenance, policy/baseline/scenario contracts,
  evaluation-host proofs and public keys, broker attestations, workflow
  provenance, and trusted-source inventory so later verification does not
  depend on rotated protected variables.
- [Hypothesis] Four local families plus the bounded durability suite do not
  establish production durability, remote transport security, or unbounded
  performance under load. Promotion remains false until the provider-host
  evidence lifecycle completes.

Within the repository boundary, the identified authority local optima are
corrected: the evaluated host—not a substitute broker probe—must produce the
session/host proof; privileged YAML and verifier code are exact-SHA bound;
evaluated code no longer executes with signing authority; symmetric forge
authority is removed; review identity, source runs, and artifact bytes are
cryptographically/content bound; GitHub credentials do not cross the storage
redirect; and evidence crosses the 90-day review window with its verification
material. Operational proof is still missing. The repository now has the
provider CLI-session Environment names, but it still has no repository-scoped
runner, no matching provider runner assignment in the current-HEAD remote
probe, no attestation/broker/signer/finalizer authority chain, and no main
protection; organization-scoped authority remains unobservable to the active
token. The repository can therefore prove only the intended fail-closed shape.
Stable promotion remains unproven until independent platform operators
provision and expose the external host attestors, keys, remaining protected
workflows/environments, and execute those boundaries. This is not a claim of a
global optimum or stable-contract readiness.

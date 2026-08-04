# Fresh-agent manual reviews

This directory is the reviewed input seam for the final fresh-agent release
aggregate. A review is authored only after both provider artifacts are
retained. The committed document is an unsigned review source, not release
authority. It is passed by repository-relative path and exact source commit to
`fresh-agent-manual-review-sign.yml`, whose protected environment produces the
signed artifact consumed by `fresh-agent-release-finalize.yml`.

The unsigned JSON source must contain:

- `run_content_hashes.codex` and `.claude`, exactly matching the two retained
  `summary.json` hashes;
- one judgment for every provider/scenario pair, with `outcome` and a non-empty
  qualitative `reason`;
- a run-bound, reviewer-authored `cost_waivers` entry only when provider cost
  is unobservable. Its `maximum_usd` must be positive and no smaller than the
  run's declared budget.

The signer also obtains canonical provenance for each provider artifact from
GitHub rather than trusting fields in the authored file. The signed payload
therefore binds repository, evaluation workflow and workflow ID, run ID and
attempt, head ref and exact SHA, event and conclusion, plus each provider
artifact ID/name/digest. The finalizer independently observes the same
coordinates and rejects any mismatch.

Do not add signature fields by hand. The signing workflow checks out the exact
review-source commit and the protected `CASEGRAPHEN_TRUSTED_VERIFIER_SHA`, then
uses the Ed25519 private key available only in the
`fresh-agent-manual-review-signer` environment. It emits schema
`casegraphen.eval.fresh_agent_manual_review.v1` with the protected
`reviewer_identity`, `reviewer_key_id`, `signature_algorithm: ed25519`, and an
`ed25519_signature` over the canonical review payload. Repository review of an
unsigned file does not substitute for this signature or the protected signer
approval.

The signer job also requires the privileged workflow's own `github.sha` to
equal `CASEGRAPHEN_TRUSTED_VERIFIER_SHA`; checking out a trusted helper does not
authorize workflow YAML from another revision. Finalizer and publisher jobs
apply the same exact-SHA boundary.

The source path must match
`docs/evals/fresh-agent/reviews/*.json`, resolve below that exact directory,
and name a regular non-symlink file. The strict parser rejects duplicate JSON
members, non-finite values such as `NaN` or `Infinity`, pre-existing signature
fields, and malformed provenance. The downstream aggregator reports missing,
duplicate, stale, unbound, or incomplete judgments and discards all review
authority on any finding. These checks prevent path substitution and ambiguous
canonical signatures; they do not make the reviewer independent without the
protected environment approval.

Do not commit provider credentials, attestation keys, session metadata, or raw
account probes. No provider API key is required or accepted. Host attestations,
the evaluation-runner host proofs, and the signed manual review are transferred
as workflow artifacts retained for the 90-day review window. Host-attestor,
broker, and reviewer private keys never enter those artifacts. The finalizer
receives only protected public-key variables and verifies their configured
SHA-256 SPKI fingerprints before checking Ed25519 signatures.

The canonical parser in `scripts/fresh-agent-release.py` rejects missing,
duplicate, stale, unbound, or incomplete judgments. A manual pass cannot
override a deterministic evaluator failure. It also requires the signed
review's identity, key ID, run hashes, and signature to match the protected
finalizer inputs exactly; if authority verification fails, judgments and cost
waivers are discarded.

The finalization workflow identifies evidence by exact workflow path, run ID,
run attempt, head SHA, artifact ID/name/digest, repository, branch, event, and
successful conclusion. A successful strict aggregate is retained as an Actions
artifact for 90 days and then packaged as a content-addressed GitHub Release
asset. That Release publication refuses overwrite and verifies a redownloaded
digest, but it is not WORM storage and remains subject to authorized GitHub
Release mutation or deletion.

The durable package retains the public PEM files and key-provenance records,
release policy, evaluator baseline, scenario manifest, evaluation-host proofs
and corresponding public keys, broker attestations, signed review, workflow
provenance, and trusted-verifier source inventory. This supports later
verification after protected variables rotate; it does not expose a provider
session or signing key.

# Fresh-agent manual reviews

This directory is the reviewed input seam for the final fresh-agent release
aggregate. A review is authored only after both provider artifacts are
retained. It is committed through normal repository review and passed to
`fresh-agent-release-finalize.yml` by repository-relative path.

The JSON document must use
`casegraphen.eval.fresh_agent_manual_review.v0` and contain:

- `run_content_hashes.codex` and `.claude`, exactly matching the two retained
  `summary.json` hashes;
- one judgment for every provider/scenario pair, with `outcome`, `reviewer`,
  and a non-empty qualitative `reason`;
- a run-bound, reviewer-authored `cost_waivers` entry only when provider cost
  is unobservable. Its `maximum_usd` must be positive and no smaller than the
  run's declared budget.

Do not commit provider credentials, attestation keys, session metadata, or raw
account probes. Host attestations are transferred as protected workflow
artifacts and verification keys are supplied only by the
`fresh-agent-release-verifier` environment.

The canonical parser in `scripts/fresh-agent-release.py` rejects missing,
duplicate, stale, unbound, or incomplete judgments. A manual pass cannot
override a deterministic evaluator failure.

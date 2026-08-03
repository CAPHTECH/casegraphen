# Issue 58 runtime pilot evidence

This directory records a local execution of
`scripts/runtime-integration-pilots.py` on 2026-08-03. It is evidence for the
experimental v0 evaluation; it is not an accepted CaseGraphen ledger entry.

- `pilot-report.json` joins adapter observations, canonical host lint and
  reconciliation results, and executable assertions.
- `process-jsonl.complete.jsonl` is the direct generic-JSONL runtime stream.
- `file-drop.complete.jsonl` is the normalized stream from the native file-drop
  runtime.
- `redesign-proposal.json` is a content-addressed, unreviewed response to the
  observed schema mismatch.
- `v0-next-version-proposal.json` records contract changes suggested by the
  two runtime boundaries without changing v0.
- `promotion-report.json` separates evidence, unknowns, and blockers and keeps
  `promotion_recommended` and `accepted` false.

Re-run into a temporary directory to verify behavior. Do not compare measured
latency byte-for-byte, because it is a runtime observation. Proposal IDs and
topology/output content addresses are deterministic for the same sources.

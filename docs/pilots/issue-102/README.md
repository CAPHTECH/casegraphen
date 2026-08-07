# Issue 102 GitHub evidence adapter dogfood pilot

This retained pilot runs `casegraphen github observe|refresh|project`
against a real, frozen capture of Issue #92 / PR #101
(`CAPHTECH/casegraphen`) — the same PR the [Issue 92 Memory Plane
pilot](../issue-92/README.md) shipped. It demonstrates exact snapshot
binding to repository/PR/base/head, three-role independence classification
on real corpus data (self-review, automated-bot, and the absence of any
independent human approval), a rate-limited CodeRabbit review made visible
rather than folded into success, and byte-equivalent replay from the
retained source bytes. It does not claim independence is proven, and it does
not write to a CaseStore: every record this pilot's commands emit carries
`accepted: false`.

See [`comparison-report.md`](comparison-report.md) for how this reproduces,
and where it extends, the manual PR-101 review record (acceptance criterion
A12). See [ADR 0031](../../adr/0031-github-evidence-observation-boundary.md)
and the [design doc](../../design/issue-102-github-evidence-adapter.md) for
the full architectural rationale.

## Retained artifacts

- `source/`: the six raw provider capture files (`gh`/`gh api graphql`
  output), byte-for-byte as captured 2026-08-06 — **frozen**, never
  re-fetched or edited. §10.1 of the design doc lists each file's SHA-256
  and the exact command that produced it;
- `capture_manifest.v0.json`: the caller-authored manifest binding each
  `source/` file to its manifest category and declared content hash;
- `expected/`: retained adapter output from a real run against the capture
  above — `pr_observation.json`, `check_evidence.json`,
  `review_findings.json`, `review_independence.json`, `refresh_result.json`
  (refreshed against itself, i.e. `head_unchanged`), `review_projection.json`.
  These are also the source of the record examples under
  `schemas/experimental/github.*.v0.example.json` — an example can never
  drift from what the Rust owner actually produces, because it *is* that
  output (§3.8 step 2 of the design doc);
- `../../../tests/fixtures/github-evidence/`: the twelve adversarial
  captures (design §10.2) built as separate mutated copies of this corpus,
  or minimal synthetic captures, never as edits to `source/`. Two
  generator scripts (`generate_pilot_derived.py`, `generate_synthetic.py`)
  reproduce them byte-for-byte; regenerating is not part of the test or
  build gate.

## Replay

```sh
casegraphen github observe \
  --manifest docs/pilots/issue-102/capture_manifest.v0.json \
  --capture-dir docs/pilots/issue-102 \
  --format json

casegraphen github project \
  --manifest docs/pilots/issue-102/capture_manifest.v0.json \
  --capture-dir docs/pilots/issue-102 \
  --format json

casegraphen github refresh \
  --manifest docs/pilots/issue-102/capture_manifest.v0.json \
  --capture-dir docs/pilots/issue-102 \
  --previous-manifest docs/pilots/issue-102/capture_manifest.v0.json \
  --previous-capture-dir docs/pilots/issue-102 \
  --format json
```

None of these open a `--store`; all three are read-only and store-free by
construction (`src/github_evidence/` never imports `native_store`). Run the
retained evidence gate:

```sh
cargo test --test github_evidence
cargo test --test experimental_schema_conformance
cargo test --test product_surface -- github
python3 scripts/experimental-schema-conformance.py --check --self-test
```

## Expected counters (the acceptance oracle)

Verified against the real, frozen capture — `tests/github_evidence.rs`'s
`observe_reproduces_the_documented_pilot_ground_truth`,
`project_pilot_has_no_blocking_findings_but_declares_the_two_residual_risks`,
and `rebuild_from_retained_source_matches_retained_expected_hashes` assert
these exactly, the last one against the retained `expected/` hashes rather
than only against another run inside the same test process:

- base `947f347f219a60775bcf71b226ce778cc8ea21f4`, head
  `c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b`; `liveness.state: "MERGED"` with
  `liveness.mergeable: "UNKNOWN"` (never coerced to a boolean); 78 changed
  files;
- two successful `check_run/quality` check evidences, plus one
  `status_context/CodeRabbit/SUCCESS` whose `description` ("Review rate
  limited") survives into `review_projection.residual_risks` rather than
  being dropped as an unmapped provider field;
- nine review threads, all resolved, zero unresolved actionable findings;
  `resolvedBy` is `rizumita` on five and `coderabbitai[bot]` on four, the
  latter sharing GitHub node id `BOT_kgDOCCSy2w` with review/comment author
  `coderabbitai` — one actor, two logins, resolved by node id, not login
  (see the comparison report for why this is load-bearing, not
  hypothetical);
- `implementation_actors.actor_ids == ["MDQ6VXNlcjc5MDUxMQ=="]` (`rizumita`
  is the PR author and every commit's author and committer); every
  `rizumita` subject classifies `self_review` despite `authorAssociation:
  MEMBER`; every `coderabbitai` subject classifies `automated_bot` via
  `__typename: "Bot"`;
- `independent_human_approvals: []`; `policy.satisfied: false` under
  `--require-independent-review`; the standing
  `independent_minds_not_observable` finding is always attached;
- `review_projection.blocking_findings: []` with `residual_risks` declaring
  both `no_independent_human_approval` and the CodeRabbit rate-limit
  description — a clean projection that still names what it did not verify;
- a fresh `github observe`/`github project` run against the retained
  `source/` bytes reproduces `pr_observation.normalized_content_hash` and
  `review_projection.projection_content_hash` exactly as retained in
  `expected/` — the delete-and-rebuild replay property (design §5,
  acceptance criterion E3). `github observe`/`github project` recompute
  everything from `--manifest`/`--capture-dir` on every invocation (no
  intermediate bundle file), so there is no derived index to separately
  delete — a fresh process already is the rebuild.

## Result

Every record this pilot's commands emit is `accepted: false`. The
independence classification never claims to prove independent minds — it
only proves this PR's own review history cannot supply it, since every
human reviewing identity is the PR's own author under a different
association label. Promotion of any of this evidence into an accepted case
fact still requires the existing gated `evidence attach` / review /
`operation_gate` flow; this pilot demonstrates observation and compact
projection only.

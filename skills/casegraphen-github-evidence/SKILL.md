---
name: casegraphen-github-evidence
description: Direct task skill for turning operator-captured GitHub issue/PR state into content-addressed observation records and a compact reviewer projection with casegraphen github observe|refresh|project. Use for authoring a capture manifest, observing a PR, checking a review basis for staleness, and projecting independent-review policy; use casegraphen-orchestrate for multi-phase routing. Store-free and proposal-only: observations are never accepted facts.
---

# GitHub state becomes evidence you can replay, not facts you accepted

Three boundary facts, stated once:

- The tool never fetches from GitHub: you run `gh` yourself and vouch for the
  capture.
- Every record these commands emit carries `accepted: false`; attaching one to
  a case space is `casegraphen-operate`'s gated `evidence attach`, not this
  skill's.
- A refresh never rebases a review basis.

## Capture first, outside the tool

Follow [references/capture.md](references/capture.md) exactly: the `gh`
commands, then the `github.capture_manifest.v0` template. The recipe is not
style — `pr`/`commits` artifacts without node ids refuse the whole capture,
while trimmed actor fields elsewhere silently downgrade classification to
`unattributed`, and neither is discoverable before a live re-capture.

## Drive

```sh
casegraphen github observe --manifest manifest.json --capture-dir . --format json --output observe.json
casegraphen github project --manifest manifest.json --capture-dir . --require-independent-review --format json --output project.json
casegraphen github refresh --manifest new-manifest.json --capture-dir new \
  --previous-manifest manifest.json --previous-capture-dir . --format json --output refresh.json
```

`refresh` takes the previous **capture** (`--previous-manifest`/
`--previous-capture-dir`), because drift detection re-normalizes it;
`--previous-observation` is optional on top of that — your *declared* basis.
When you pass it, it must be the bare `result.pr_observation` record extracted
from the observe report (the report wrapper is refused), and it must equal the
re-normalized previous capture byte-for-byte.

## Report or gate

`--require-independent-review` alone is a report: an unmet policy is a domain
finding on a successful exit 0. `--strict` (all three subcommands) maps domain
findings to exit 2 with byte-identical report output; tool failure stays
exit 1. The CI gate is therefore the pair:

```sh
casegraphen github project --manifest manifest.json --capture-dir . \
  --require-independent-review --strict --format json --output project.json
```

Omit `--strict` when the projection artifact is the deliverable and a blocked
policy must not fail the step that renders it; pass it when the exit code is
the decision.

## What the exit codes mean

Hard refusals (exit 1) mean the capture or call is wrong — fix it and rerun,
guided by the refusal's own `findings`. Domain findings (exit 0, or 2 under
`--strict`) mean the tool worked and the answer carries an obstruction — act
on the answer, do not fix the call. The refusals that cost a live re-capture
rather than a free retry are tabulated at the end of
[references/capture.md](references/capture.md).

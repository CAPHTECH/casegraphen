#!/bin/sh
# CI gate: exits nonzero unless PR #101 (earlier capture) carries an approving
# review from an independent human bound to the observed head commit.
# The verdict is carried entirely by the exit code: --strict makes the
# require-independent-review blocking finding fatal (exit 2); no JSON parsing needed.
DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/../bin/casegraphen" github project \
  --capture-dir "$DIR" \
  --manifest "$DIR/manifest.json" \
  --require-independent-review \
  --strict \
  --format json \
  --output "$DIR/ci-gate-report.json"

#!/bin/sh
# CI gate: nonzero exit unless PR 101 has an approving review from an
# independent human bound to the observed head (earlier capture).
# --strict turns the blocking domain finding into a nonzero exit, so the
# gate works by exit code alone; stdout is discarded via --output.
DIR="$(cd "$(dirname "$0")" && pwd)"
exec /private/tmp/claude-501/-Users-rizumita-Workspace-casegraphen/3c964d9b-0250-4c0a-b985-e7090f8a4f0c/scratchpad/probe/bin/casegraphen \
  github project \
  --capture-dir "$DIR" \
  --manifest "$DIR/manifest.json" \
  --require-independent-review \
  --strict \
  --format json \
  --output /dev/null

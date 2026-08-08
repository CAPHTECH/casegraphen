#!/bin/sh
# Pre-tag gate worker: the declared version must match the version being
# tagged. Change `expected` to whatever release the case space is deciding on.
#
# `actual` is a pinned fixture, not read from this repository's own
# Cargo.toml (#138): reading the crate's own real, ever-advancing version
# made the walkthrough's "declared version does not yet match the tag"
# transcript rot at every release that moved Cargo.toml. The lesson this
# worker exists to teach — a failing gate is a domain finding (§10), and
# editing the pinned script does not fool identity re-measurement (§11) —
# does not depend on reading a real file, so the version is pinned instead.
set -eu

expected=0.9.0
actual=0.8.0

if [ "$actual" != "$expected" ]; then
  printf 'tag-dry-run FAILED: declared version is %s, tag would be v%s\n' "$actual" "$expected"
  exit 1
fi

printf 'tag-dry-run ok: declared version is %s\n' "$actual"

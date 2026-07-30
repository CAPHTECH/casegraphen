#!/bin/sh
# Pre-tag gate worker: the shipped crate version must be the version being
# tagged. Change `expected` to whatever release the case space is deciding on.
set -eu

expected=0.9.0
actual=$(/usr/bin/sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | /usr/bin/head -1)

if [ "$actual" != "$expected" ]; then
  printf 'tag-dry-run FAILED: Cargo.toml declares %s, tag would be v%s\n' "$actual" "$expected"
  exit 1
fi

printf 'tag-dry-run ok: Cargo.toml declares %s\n' "$actual"

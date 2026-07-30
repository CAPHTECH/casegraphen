#!/bin/sh
# Release gate worker: every shipped schema file must carry an $id.
#
# The worker environment is cleared and PATH is never allowlisted, so every
# external program is called by absolute path. Paths inside the repository are
# relative to the binding's working_directory.
set -eu

dir=schemas/casegraphen
missing=0
count=0
for file in "$dir"/*.schema.json; do
  count=$((count + 1))
  if ! /usr/bin/grep -q '"\$id"' "$file"; then
    printf 'missing $id: %s\n' "$file"
    missing=$((missing + 1))
  fi
done

if [ "$missing" -ne 0 ]; then
  printf 'schema-id-gate FAILED: %d of %d schemas carry no $id\n' "$missing" "$count"
  exit 1
fi

printf 'schema-id-gate ok: %d schemas carry $id\n' "$count"

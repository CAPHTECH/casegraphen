#!/bin/sh
# Executable read-only Skill example. CI supplies a temporary store.
set -eu

casegraphen_bin=$1
repository_root=$2
fixture_store=$3
case_space_id=case_space:casegraphen-release-0-9-0

"$casegraphen_bin" lift native --store "$fixture_store" \
  --input "$repository_root/docs/guides/release-decision/genesis.case.space.json" \
  --revision-id revision:skill-conformance --format json \
  --output "$fixture_store/lift.json"
"$casegraphen_bin" space inspect --store "$fixture_store" \
  --case-space-id "$case_space_id" --format json --output "$fixture_store/inspect.json"
"$casegraphen_bin" space frontier --store "$fixture_store" \
  --case-space-id "$case_space_id" --format json --output "$fixture_store/frontier.json"
"$casegraphen_bin" space reason --store "$fixture_store" \
  --case-space-id "$case_space_id" --format text --output "$fixture_store/reason.txt"
"$casegraphen_bin" space validate --store "$fixture_store" \
  --case-space-id "$case_space_id" --format json --output "$fixture_store/validate.json"
"$casegraphen_bin" obstruction list --store "$fixture_store" \
  --case-space-id "$case_space_id" --format json --output "$fixture_store/obstructions.json"

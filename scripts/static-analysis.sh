#!/bin/sh
# Release quality gate for CaseGraphen.
#
# Every check here is a gate, not a report: the script exits non-zero on the
# first failure. CI runs exactly this script, so a green local run and a green
# CI run mean the same thing.
#
# Requires: cargo (with rustfmt and clippy), python3 with the jsonschema
# package (the integration tests shell out to `python3 -m jsonschema` to
# validate generated reports against the shipped contracts).
set -eu

say() { printf '\n== %s\n' "$1"; }

say 'formatting'
cargo fmt --all --check

say 'lints (warnings are failures)'
cargo clippy --all-targets --locked -- -D warnings

say 'tests'
cargo test --locked

say 'packaging (proves the crate publishes and builds standalone)'
# A dirty tree is normal while developing; the check is that the crate packages
# and compiles standalone, not that the tree is committed. CI checks out clean,
# so it exercises the strict form.
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
  printf 'note: working tree is dirty, packaging with --allow-dirty\n'
  cargo package --locked --allow-dirty
else
  cargo package --locked
fi

say 'schema and example inventory'
python3 - <<'PY'
import json
import pathlib
import re
import sys

schema_dir = pathlib.Path("schemas/casegraphen")

# Report envelopes are validated through the shared envelope plus
# report-schema-aliases.json rather than through a fixture of their own, so
# they legitimately ship without an example. Everything else must have one.
envelope_only = {
    "case.report",
    "native-cli.report",
    "native.morphism-log-entry",
    "workflow.operation.report",
}

problems = []
for schema_path in sorted(schema_dir.glob("*.schema.json")):
    stem = schema_path.name[: -len(".schema.json")]
    schema = json.loads(schema_path.read_text())
    if "$id" not in schema:
        problems.append(f"{schema_path}: missing $id")
    example = schema_dir / f"{stem}.example.json"
    if stem in envelope_only:
        continue
    if not example.exists():
        problems.append(f"{schema_path}: no {example.name} (add one or list it as envelope-only)")

aliases = json.loads((schema_dir / "report-schema-aliases.json").read_text())
for alias in aliases.get("aliases", []):
    target = alias.get("target_schema_id")
    if not any(
        json.loads(p.read_text()).get("$id") == target
        for p in schema_dir.glob("*.schema.json")
    ):
        problems.append(f"report-schema-aliases.json: target {target} resolves to no schema file")
    try:
        re.compile(alias.get("schema_id_pattern", ""))
    except re.error as error:
        # ECMAScript-only constructs are allowed here (documented in the audit),
        # so report rather than fail on Python's inability to compile them.
        print(f"note: alias pattern not compilable by Python re: {error}", file=sys.stderr)

if problems:
    for problem in problems:
        print(f"FAIL {problem}", file=sys.stderr)
    raise SystemExit(1)

print(f"ok: {len(list(schema_dir.glob('*.schema.json')))} schemas, aliases resolve")
PY

say 'size report (informational)'
find src -name '*.rs' | xargs wc -l | tail -1

printf '\nall gates passed\n'

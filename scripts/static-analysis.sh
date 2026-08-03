#!/bin/sh
# Release quality gate for CaseGraphen.
#
# Every check here is a gate, not a report: the script exits non-zero on the
# first failure. CI runs exactly this script, so a green local run and a green
# CI run mean the same thing.
#
# Requires: cargo (with rustfmt and clippy), python3 with
# jsonschema==4.26.0 (the version pinned in CI; integration tests shell out to
# `python3 -m jsonschema` to validate generated reports against the shipped
# contracts).
set -eu

say() { printf '\n== %s\n' "$1"; }

# rust-toolchain.toml pins the toolchain so that clippy agrees between a local
# run and CI. RUSTUP_TOOLCHAIN in the environment silently overrides that file,
# which is how the first CI run of this repository failed on a lint the local
# toolchain no longer emits. Warn rather than fail: an override is legitimate
# when deliberately testing another toolchain.
pinned=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml 2>/dev/null || true)
active=$(rustc --version 2>/dev/null | cut -d' ' -f2)
cargo_msrv=$(sed -n 's/^rust-version *= *"\(.*\)"/\1/p' Cargo.toml 2>/dev/null || true)
normalized_pin=${pinned%.0}
if [ -z "$pinned" ] || [ -z "$cargo_msrv" ] || [ "$normalized_pin" != "$cargo_msrv" ]; then
  printf 'error: rust-toolchain.toml (%s) and Cargo.toml rust-version (%s) disagree.\n' \
    "${pinned:-missing}" "${cargo_msrv:-missing}" >&2
  exit 1
fi
for workflow in .github/workflows/*.yml; do
  workflow_pins=$(awk '
    /uses: dtolnay\/rust-toolchain@/ {
      ref = $0
      sub(/^.*uses: dtolnay\/rust-toolchain@/, "", ref)
      sub(/[[:space:]].*$/, "", ref)
      if (ref ~ /^[0-9]+\.[0-9]+\.[0-9]+$/) {
        print ref
        awaiting_toolchain = 0
      } else {
        awaiting_toolchain = 1
      }
      next
    }
    awaiting_toolchain && /^[[:space:]]+toolchain:[[:space:]]*/ {
      value = $0
      sub(/^.*toolchain:[[:space:]]*/, "", value)
      sub(/[[:space:]#].*$/, "", value)
      print value
      awaiting_toolchain = 0
      next
    }
    awaiting_toolchain && /^[[:space:]]*-[[:space:]]+(uses:|name:|run:)/ {
      print "missing-toolchain-input"
      awaiting_toolchain = 0
    }
    END {
      if (awaiting_toolchain) {
        print "missing-toolchain-input"
      }
    }
  ' "$workflow")
  workflow_action_count=$(awk '/uses: dtolnay\/rust-toolchain@/ { count += 1 } END { print count + 0 }' "$workflow")
  workflow_pin_count=$(printf '%s\n' "$workflow_pins" | awk 'NF { count += 1 } END { print count + 0 }')
  if [ "$workflow_action_count" != "$workflow_pin_count" ]; then
    printf 'error: %s must declare toolchain: %s when dtolnay/rust-toolchain is SHA-pinned.\n' \
      "$workflow" "$pinned" >&2
    exit 1
  fi
  for workflow_pin in $workflow_pins; do
    if [ "$workflow_pin" != "$pinned" ]; then
      printf 'error: %s pins Rust %s but rust-toolchain.toml pins %s.\n' \
        "$workflow" "$workflow_pin" "$pinned" >&2
      exit 1
    fi
  done
done
if [ -n "$pinned" ] && [ -n "$active" ] && [ "$pinned" != "$active" ]; then
  printf 'warning: rust %s is active but rust-toolchain.toml pins %s.\n' "$active" "$pinned"
  printf '         CI runs %s; re-run with `rustup run %s sh scripts/static-analysis.sh`.\n' \
    "$pinned" "$pinned"
fi

say 'toolchain contract'
printf 'declared MSRV: %s (toolchain pin %s)\n' "$cargo_msrv" "$pinned"
rustc --version --verbose
cargo clippy --version

say 'formatting'
cargo fmt --all --check

say 'installer smoke test'
sh scripts/install-smoke-test.sh

say 'Skill conformance'
python3 scripts/skill-conformance.py --check

say 'Graph Engineering product-surface conformance'
python3 scripts/product-surface-conformance.py
python3 scripts/fresh-agent-workflow-conformance.py

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

say 'experimental contract conformance'
python3 scripts/experimental-schema-conformance.py --check --self-test

say 'size report (informational)'
find src -name '*.rs' | xargs wc -l | tail -1

printf '\nall gates passed\n'

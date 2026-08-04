#!/bin/sh
# Verify that install.sh targets both user-level agent homes and never the
# project from which it is invoked. A fake cargo keeps this test fast and
# prevents it from modifying the developer's actual Cargo installation.
set -eu

repository_dir=$(cd "$(dirname "$0")/.." && pwd)
test_dir=$(mktemp -d "${TMPDIR:-/tmp}/casegraphen-install-test.XXXXXX")
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM

mkdir -p "$test_dir/bin" "$test_dir/project"
printf '#!/bin/sh\nexit 0\n' >"$test_dir/bin/cargo"
chmod +x "$test_dir/bin/cargo"

install_output="$test_dir/install-output.txt"
(
  cd "$test_dir/project"
  HOME="$test_dir/home" CARGO_HOME="$test_dir/cargo-home" \
    PATH="$test_dir/bin:/usr/bin:/bin" \
    sh "$repository_dir/install.sh"
) >"$install_output"
cat "$install_output"

mcp_binary="$test_dir/cargo-home/bin/casegraphen-mcp"
grep -F "codex mcp add casegraphen -- '$mcp_binary'" "$install_output" >/dev/null
grep -F "claude mcp add --scope user casegraphen -- '$mcp_binary'" "$install_output" >/dev/null
grep -F 'codex mcp get casegraphen' "$install_output" >/dev/null
grep -F 'claude mcp get casegraphen' "$install_output" >/dev/null
grep -F "$repository_dir/docs/guides/mcp-operational-host.md" "$install_output" >/dev/null

for runtime in .claude .codex; do
  for skill in casegraphen-operate casegraphen-design casegraphen-audit casegraphen-integrate; do
    installed="$test_dir/home/$runtime/skills/$skill/SKILL.md"
    if [ ! -f "$installed" ]; then
      printf 'missing installed skill: %s\n' "$installed" >&2
      exit 1
    fi
  done
  audit_reference="$test_dir/home/$runtime/skills/casegraphen-audit/references/run-audit.md"
  audit_agent="$test_dir/home/$runtime/skills/casegraphen-audit/agents/openai.yaml"
  audit_schema="$test_dir/home/$runtime/skills/casegraphen-audit/references/runtime.node_report.schema.json"
  for installed_asset in "$audit_reference" "$audit_agent" "$audit_schema"; do
    if [ ! -f "$installed_asset" ]; then
      printf 'missing installed skill asset: %s\n' "$installed_asset" >&2
      exit 1
    fi
  done
  cmp "$repository_dir/schemas/experimental/runtime.node_report.schema.json" "$audit_schema"
  integrate_reference="$test_dir/home/$runtime/skills/casegraphen-integrate/references/generic-jsonl.md"
  integrate_agent="$test_dir/home/$runtime/skills/casegraphen-integrate/agents/openai.yaml"
  for installed_asset in "$integrate_reference" "$integrate_agent"; do
    if [ ! -f "$installed_asset" ]; then
      printf 'missing installed skill asset: %s\n' "$installed_asset" >&2
      exit 1
    fi
  done
  design_reference="$test_dir/home/$runtime/skills/casegraphen-design/references/contracts-and-outputs.md"
  design_agent="$test_dir/home/$runtime/skills/casegraphen-design/agents/openai.yaml"
  design_schema="$test_dir/home/$runtime/skills/casegraphen-design/references/execution.topology.v0.schema.json"
  design_contract="$test_dir/home/$runtime/skills/casegraphen-design/references/execution-topology-contract.md"
  for installed_asset in "$design_reference" "$design_agent" "$design_schema" "$design_contract"; do
    if [ ! -f "$installed_asset" ]; then
      printf 'missing installed skill asset: %s\n' "$installed_asset" >&2
      exit 1
    fi
  done
  cmp "$repository_dir/schemas/experimental/execution.topology.v0.schema.json" "$design_schema"
  cmp "$repository_dir/docs/design/execution-topology-contract.md" "$design_contract"
done

if [ -e "$test_dir/project/.claude" ]; then
  printf 'installer wrote into the invoking project\n' >&2
  exit 1
fi

if HOME="$test_dir/home" CARGO_HOME="$test_dir/cargo-home" \
  PATH="$test_dir/bin:/usr/bin:/bin" \
  sh "$repository_dir/install.sh" --user >/dev/null 2>&1; then
  printf 'installer unexpectedly accepted the removed --user option\n' >&2
  exit 1
fi

printf 'install smoke test passed\n'

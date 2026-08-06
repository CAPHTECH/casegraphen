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

# install_binary() must say what it found on PATH before replacing it, and
# what checkout (with commit) it is installing from — see #105.
grep -F 'no existing casegraphen found on PATH' "$install_output" >/dev/null
source_commit=$(git -C "$repository_dir" rev-parse HEAD)
grep -F "== installing the casegraphen binary from $repository_dir (commit $source_commit)" \
  "$install_output" >/dev/null

# With an existing `casegraphen` on PATH, its version is reported before
# install.sh replaces it.
mkdir -p "$test_dir/existing-bin"
printf '#!/bin/sh\nif [ "$1" = "--version" ]; then printf "casegraphen 0.1.0-fake\\n"; exit 0; fi\nexit 1\n' \
  >"$test_dir/existing-bin/casegraphen"
chmod +x "$test_dir/existing-bin/casegraphen"

existing_output="$test_dir/install-output-existing.txt"
(
  cd "$test_dir/project"
  HOME="$test_dir/home" CARGO_HOME="$test_dir/cargo-home" \
    PATH="$test_dir/existing-bin:$test_dir/bin:/usr/bin:/bin" \
    sh "$repository_dir/install.sh"
) >"$existing_output"
grep -F "existing casegraphen on PATH ($test_dir/existing-bin/casegraphen): casegraphen 0.1.0-fake" \
  "$existing_output" >/dev/null

# --root scopes the binary install without disturbing CARGO_HOME's default,
# and the MCP setup output reflects the scoped path.
root_output="$test_dir/install-output-root.txt"
custom_root="$test_dir/custom-root"
(
  cd "$test_dir/project"
  HOME="$test_dir/home" CARGO_HOME="$test_dir/cargo-home" \
    PATH="$test_dir/bin:/usr/bin:/bin" \
    sh "$repository_dir/install.sh" --root "$custom_root"
) >"$root_output"
root_mcp_binary="$custom_root/bin/casegraphen-mcp"
grep -F "codex mcp add casegraphen -- '$root_mcp_binary'" "$root_output" >/dev/null
grep -F "claude mcp add --scope user casegraphen -- '$root_mcp_binary'" "$root_output" >/dev/null

for runtime in .claude .codex; do
  for skill in casegraphen-orchestrate casegraphen-operate casegraphen-design casegraphen-audit casegraphen-integrate casegraphen-memory-query casegraphen-memory-curate casegraphen-memory-audit casegraphen-github-evidence; do
    installed="$test_dir/home/$runtime/skills/$skill/SKILL.md"
    if [ ! -f "$installed" ]; then
      printf 'missing installed skill: %s\n' "$installed" >&2
      exit 1
    fi
  done
  for skill in casegraphen-orchestrate casegraphen-operate casegraphen-design casegraphen-audit casegraphen-integrate casegraphen-memory-query casegraphen-memory-curate casegraphen-memory-audit casegraphen-github-evidence; do
    installed_agent="$test_dir/home/$runtime/skills/$skill/agents/openai.yaml"
    if [ ! -f "$installed_agent" ]; then
      printf 'missing installed skill agent metadata: %s\n' "$installed_agent" >&2
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
  orchestrate_reference="$test_dir/home/$runtime/skills/casegraphen-orchestrate/references/routing.md"
  orchestrate_handoff="$test_dir/home/$runtime/skills/casegraphen-orchestrate/references/handoff.md"
  orchestrate_schema="$test_dir/home/$runtime/skills/casegraphen-orchestrate/references/skill.orchestration_handoff.v0.schema.json"
  orchestrate_example="$test_dir/home/$runtime/skills/casegraphen-orchestrate/references/skill.orchestration_handoff.v0.example.json"
  for installed_asset in "$orchestrate_reference" "$orchestrate_handoff" "$orchestrate_schema" "$orchestrate_example"; do
    if [ ! -f "$installed_asset" ]; then
      printf 'missing installed process skill asset: %s\n' "$installed_asset" >&2
      exit 1
    fi
  done
  cmp "$repository_dir/schemas/experimental/skill.orchestration_handoff.v0.schema.json" "$orchestrate_schema"
  cmp "$repository_dir/schemas/experimental/skill.orchestration_handoff.v0.example.json" "$orchestrate_example"
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

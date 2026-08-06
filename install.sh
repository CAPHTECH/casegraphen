#!/bin/sh
# Install CaseGraphen: the `casegraphen` binary, and the agent skills that
# drive it.
#
# The script builds and copies from its own location, so it works from a
# repository checkout and from the unpacked crate under ~/.cargo/registry alike
# — you never have to work out a relative path.
#
#   sh install.sh            # binary, and skills into ~/.claude and ~/.codex
set -eu

source_dir=$(cd "$(dirname "$0")" && pwd)

# One entry per runtime that reads a skill from <home>/skills. The installer
# creates missing homes so both runtimes receive the shipped skills.
agent_homes() {
  for home in "$HOME/.claude" "$HOME/.codex"; do
    printf '%s\n' "$home"
  done
}

install_binary() {
  printf '== installing the casegraphen binary from %s\n' "$source_dir"
  cargo install --path "$source_dir" --locked
}

install_skills_into() {
  target=$1
  mkdir -p "$target"

  installed=0
  for skill in "$source_dir"/skills/*/SKILL.md; do
    [ -f "$skill" ] || continue
    name=$(basename "$(dirname "$skill")")

    if [ -L "$target/$name" ]; then
      printf 'skipped %s: %s is a symlink and already tracks its source\n' "$name" "$target/$name"
      continue
    fi
    if [ -d "$target/$name" ]; then
      printf 'replacing existing %s\n' "$target/$name"
      rm -rf "$target/$name"
    fi

    cp -R "$(dirname "$skill")" "$target/$name"
    if [ "$name" = casegraphen-design ]; then
      cp "$source_dir/schemas/experimental/execution.topology.v0.schema.json" \
        "$target/$name/references/execution.topology.v0.schema.json"
      cp "$source_dir/docs/design/execution-topology-contract.md" \
        "$target/$name/references/execution-topology-contract.md"
    fi
    if [ "$name" = casegraphen-audit ]; then
      cp "$source_dir/schemas/experimental/runtime.node_report.schema.json" \
        "$target/$name/references/runtime.node_report.schema.json"
    fi
    if [ "$name" = casegraphen-orchestrate ]; then
      # The process Skill consumes the canonical handoff contract. Copying it
      # at install time prevents the bundled reference from drifting from the
      # schema inventoried and validated by this source tree.
      cp "$source_dir/schemas/experimental/skill.orchestration_handoff.v0.schema.json" \
        "$target/$name/references/skill.orchestration_handoff.v0.schema.json"
      cp "$source_dir/schemas/experimental/skill.orchestration_handoff.v0.example.json" \
        "$target/$name/references/skill.orchestration_handoff.v0.example.json"
    fi
    printf 'installed %s\n' "$name"
    installed=$((installed + 1))
  done

  printf '%d skill(s) written to %s\n' "$installed" "$target"
}

installed_binary_path() {
  name=$1
  if [ -n "${CARGO_INSTALL_ROOT:-}" ]; then
    printf '%s/bin/%s\n' "$CARGO_INSTALL_ROOT" "$name"
    return
  fi
  if [ -n "${CARGO_HOME:-}" ]; then
    printf '%s/bin/%s\n' "$CARGO_HOME" "$name"
    return
  fi

  resolved=$(command -v "$name" 2>/dev/null || true)
  case "$resolved" in
    /*)
      printf '%s\n' "$resolved"
      return
      ;;
  esac

  printf '%s/.cargo/bin/%s\n' "$HOME" "$name"
}

print_mcp_setup() {
  mcp_binary=$(installed_binary_path casegraphen-mcp)

  printf '\n== configure the CaseGraphen MCP reference adapter\n'
  printf 'Codex (user configuration):\n'
  printf "  codex mcp add casegraphen -- '%s'\n" "$mcp_binary"
  printf '  codex mcp get casegraphen\n'
  printf '\nClaude Code (user configuration):\n'
  printf "  claude mcp add --scope user casegraphen -- '%s'\n" "$mcp_binary"
  printf '  claude mcp get casegraphen\n'
  printf '\nThe reference adapter provides local graph linting and fails closed for\n'
  printf 'operations that require an external decision or resource owner.\n'
  printf 'For the durable authenticated operational host, follow:\n'
  printf '  %s/docs/guides/mcp-operational-host.md\n' "$source_dir"
}

case "$#" in
  0)
    if [ -z "${HOME:-}" ]; then
      printf 'HOME must be set to install agent skills\n' >&2
      exit 1
    fi
    install_binary
    printf '\n== installing skills\n'
    agent_homes | while IFS= read -r home; do
      install_skills_into "$home/skills"
    done
    print_mcp_setup
    ;;
  *)
    printf 'usage: %s\n' "$0" >&2
    exit 2
    ;;
esac

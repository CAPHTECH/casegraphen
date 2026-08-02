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
    printf 'installed %s\n' "$name"
    installed=$((installed + 1))
  done

  printf '%d skill(s) written to %s\n' "$installed" "$target"
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
    ;;
  *)
    printf 'usage: %s\n' "$0" >&2
    exit 2
    ;;
esac

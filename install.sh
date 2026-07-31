#!/bin/sh
# Install CaseGraphen: the `casegraphen` binary, and the agent skills that
# drive it.
#
# The script builds and copies from its own location, so it works from a
# repository checkout and from the unpacked crate under ~/.cargo/registry alike
# — you never have to work out a relative path.
#
#   sh install.sh            # binary, and skills into ./.claude/skills
#   sh install.sh --user     # binary, and skills into the skills directory of
#                            # every agent home that exists: ~/.claude, ~/.codex
set -eu

source_dir=$(cd "$(dirname "$0")" && pwd)

# The agent homes --user installs into: one entry per runtime that reads a skill
# from <home>/skills. Adding a runtime is one entry here.
existing_agent_homes() {
  for home in "$HOME/.claude" "$HOME/.codex"; do
    [ -d "$home" ] || continue
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
    printf 'installed %s\n' "$name"
    installed=$((installed + 1))
  done

  printf '%d skill(s) written to %s\n' "$installed" "$target"
}

case "${1:-}" in
  "")
    install_binary
    printf '\n== installing skills\n'
    install_skills_into "$PWD/.claude/skills"
    ;;
  --user)
    # Checked before the build so that a machine with no agent home fails in
    # seconds rather than after a full compile.
    if [ -z "$(existing_agent_homes)" ]; then
      printf 'no agent home found: neither %s nor %s exists\n' "$HOME/.claude" "$HOME/.codex" >&2
      exit 1
    fi
    install_binary
    printf '\n== installing skills\n'
    existing_agent_homes | while IFS= read -r home; do
      install_skills_into "$home/skills"
    done
    ;;
  *)
    printf 'usage: %s [--user]\n' "$0" >&2
    exit 2
    ;;
esac

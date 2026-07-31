#!/bin/sh
# Install the CaseGraphen agent skills into a skills directory an agent reads.
#
# The script copies from its own location, so it works from a repository
# checkout and from the unpacked crate under ~/.cargo/registry alike — you never
# have to work out a relative path.
#
#   sh install.sh            # into ./.claude/skills (this project)
#   sh install.sh --user     # into the skills directory of every agent home
#                            # that exists: ~/.claude, ~/.codex
set -eu

source_dir=$(cd "$(dirname "$0")" && pwd)

install_into() {
  target=$1
  mkdir -p "$target"

  installed=0
  for skill in "$source_dir"/*/SKILL.md; do
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
    install_into "$PWD/.claude/skills"
    ;;
  --user)
    found=0
    for home in "$HOME/.claude" "$HOME/.codex"; do
      [ -d "$home" ] || continue
      found=$((found + 1))
      install_into "$home/skills"
    done
    if [ "$found" -eq 0 ]; then
      printf 'no agent home found: neither %s nor %s exists\n' "$HOME/.claude" "$HOME/.codex" >&2
      exit 1
    fi
    ;;
  *)
    printf 'usage: %s [--user]\n' "$0" >&2
    exit 2
    ;;
esac

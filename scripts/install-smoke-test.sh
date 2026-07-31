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

(
  cd "$test_dir/project"
  HOME="$test_dir/home" PATH="$test_dir/bin:/usr/bin:/bin" \
    sh "$repository_dir/install.sh"
)

for runtime in .claude .codex; do
  installed="$test_dir/home/$runtime/skills/casegraphen-operate/SKILL.md"
  if [ ! -f "$installed" ]; then
    printf 'missing installed skill: %s\n' "$installed" >&2
    exit 1
  fi
done

if [ -e "$test_dir/project/.claude" ]; then
  printf 'installer wrote into the invoking project\n' >&2
  exit 1
fi

if HOME="$test_dir/home" PATH="$test_dir/bin:/usr/bin:/bin" \
  sh "$repository_dir/install.sh" --user >/dev/null 2>&1; then
  printf 'installer unexpectedly accepted the removed --user option\n' >&2
  exit 1
fi

printf 'install smoke test passed\n'

# Skills

Agent skills for **using** CaseGraphen. They are shipped here rather than under
`.claude/` because they are for consumers of the CLI, not for developing this
crate. (The repository's own development skills stay in `.claude/skills/`.)

| Skill | Use when |
|---|---|
| [`casegraphen-operate`](casegraphen-operate/SKILL.md) | Driving work through a case space: lifting one, reading readiness, applying morphisms, attaching and promoting evidence, dispatching workers with `run --step` |

## Install

From a checkout of this repository, run the installer from the project you want
to use the skill in:

```sh
cd /path/to/your/project
sh /path/to/casegraphen/skills/install.sh            # into ./.claude/skills
sh /path/to/casegraphen/skills/install.sh --user     # into ~/.claude/skills
```

The script copies from its own location, so it does not care where you run it
from. It replaces a previous install of the same skill, leaves a symlink alone if
you have linked the skill instead, and touches nothing else.

If you installed the binary with `cargo install casegraphen` rather than cloning,
the same script is in the unpacked crate source under
`~/.cargo/registry/src/<index>/casegraphen-<version>/skills/`.

Copying by hand is fine too — the skill is plain Markdown:

```sh
cp -R /path/to/casegraphen/skills/casegraphen-operate .claude/skills/
```

Claude Code discovers skills in `.claude/skills/` (project) and
`~/.claude/skills/` (user), and picks up a new one without a restart. Other agent
runtimes read `SKILL.md` directly; its frontmatter carries the name and the
description used to decide relevance.

Each skill's `references/` files are loaded on demand, so `SKILL.md` stays short
enough to keep in context.

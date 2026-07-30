# Skills

Agent skills for **using** CaseGraphen. They are shipped here rather than under
`.claude/` because they are for consumers of the CLI, not for developing this
crate. (The repository's own development skills stay in `.claude/skills/`.)

| Skill | Use when |
|---|---|
| [`casegraphen-operate`](casegraphen-operate/SKILL.md) | Driving work through a case space: lifting one, reading readiness, applying morphisms, attaching and promoting evidence, dispatching workers with `run --step` |

## Install

Copy the skill directory into your project or your user-level skills directory:

```sh
mkdir -p .claude/skills
cp -R path/to/casegraphen/skills/casegraphen-operate .claude/skills/
```

Claude Code discovers skills in `.claude/skills/` (project) and
`~/.claude/skills/` (user). Other agent runtimes read `SKILL.md` directly; the
frontmatter carries the name and the description used to decide relevance.

Each skill's `references/` files are loaded on demand, so `SKILL.md` stays short
enough to keep in context.

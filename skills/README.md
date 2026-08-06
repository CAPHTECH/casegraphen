# Skills

Agent skills for **using** CaseGraphen. They are shipped here rather than under
`.claude/` because they are for consumers of the CLI, not for developing this
crate. (The repository's own development skills stay in `.claude/skills/`.)

The surface has two layers. Direct task skills own one bounded activity and may
always be invoked without the process layer. `casegraphen-orchestrate` is the
process skill for multi-phase or ambiguous work; it selects the next task skill,
carries an exact handoff, and returns at review or authority seams. It cannot
accept, review, silently rebase, or broaden authority.

| Skill | Use when |
|---|---|
| [`casegraphen-orchestrate`](casegraphen-orchestrate/SKILL.md) | Routing end-to-end or ambiguous work across task skills while preserving exact revisions, artifacts, unresolved evidence, and non-delegable seams |
| [`casegraphen-operate`](casegraphen-operate/SKILL.md) | Driving work through a case space: lifting one, reading readiness, applying morphisms, attaching and promoting evidence, dispatching workers with `run --step`, and governing an agent runtime's graph by recording what it produced as reviewable evidence |
| [`casegraphen-design`](casegraphen-design/SKILL.md) | Turning a problem statement into an unreviewed, typed, linted execution-topology proposal before selecting or running a runtime |
| [`casegraphen-audit`](casegraphen-audit/SKILL.md) | Auditing static topology and canonical planned-versus-reported completeness without promoting runtime declarations |
| [`casegraphen-integrate`](casegraphen-integrate/SKILL.md) | Ingesting generic external-runtime JSONL, reconciling canonical completeness, and stopping at unreviewed evidence/morphism proposals |
| [`casegraphen-memory-query`](casegraphen-memory-query/SKILL.md) | Reading revision-bound accepted project memory while preserving sources, authority, time, conflict, and projection loss |
| [`casegraphen-memory-curate`](casegraphen-memory-curate/SKILL.md) | Creating immutable-source-backed, scoped, temporal, unreviewed claim and relation proposals without accepting them |
| [`casegraphen-memory-audit`](casegraphen-memory-audit/SKILL.md) | Auditing provenance, authority laundering, stale memory, conflicts, scope leakage, loss, and index replay equivalence without mutation |

## Install

[`install.sh`](../install.sh) at the repository root installs the `casegraphen`
binary and these skills together — the skill is instructions for a CLI, so
installing one without the other leaves you with half of it:

```sh
sh /path/to/casegraphen/install.sh
```

The binary goes where `cargo install` puts it (`~/.cargo/bin` unless you have
moved it), built from the same source tree the skills are copied from, so the
documented command surface is the installed one.

The skills are installed into both `~/.claude/skills` and `~/.codex/skills`.
The installer creates either directory when it does not exist and never uses
the current project's `.claude/skills` as an install target.

The script builds and copies from its own location, so it does not care where you
run it from. It replaces a previous install of the same skill, leaves a symlink
alone if you have linked the skill instead, and touches nothing else.

If you installed the binary with `cargo install casegraphen` rather than cloning,
the same script is in the unpacked crate source under
`~/.cargo/registry/src/<index>/casegraphen-<version>/`.

Copying by hand is fine too, but copy each whole directory so its on-demand
references and agent metadata stay attached:

```sh
cp -R /path/to/casegraphen/skills/casegraphen-orchestrate ~/.claude/skills/
cp -R /path/to/casegraphen/skills/casegraphen-orchestrate ~/.codex/skills/
cp -R /path/to/casegraphen/skills/casegraphen-operate ~/.claude/skills/
cp -R /path/to/casegraphen/skills/casegraphen-operate ~/.codex/skills/
cp -R /path/to/casegraphen/skills/casegraphen-design ~/.claude/skills/
cp -R /path/to/casegraphen/skills/casegraphen-design ~/.codex/skills/
cp -R /path/to/casegraphen/skills/casegraphen-audit ~/.claude/skills/
cp -R /path/to/casegraphen/skills/casegraphen-audit ~/.codex/skills/
cp -R /path/to/casegraphen/skills/casegraphen-integrate ~/.claude/skills/
cp -R /path/to/casegraphen/skills/casegraphen-integrate ~/.codex/skills/
cp -R /path/to/casegraphen/skills/casegraphen-memory-query ~/.claude/skills/
cp -R /path/to/casegraphen/skills/casegraphen-memory-query ~/.codex/skills/
cp -R /path/to/casegraphen/skills/casegraphen-memory-curate ~/.claude/skills/
cp -R /path/to/casegraphen/skills/casegraphen-memory-curate ~/.codex/skills/
cp -R /path/to/casegraphen/skills/casegraphen-memory-audit ~/.claude/skills/
cp -R /path/to/casegraphen/skills/casegraphen-memory-audit ~/.codex/skills/
```

Claude Code discovers skills in `.claude/skills/` (project) and
`~/.claude/skills/` (user), and picks up a new one without a restart. Other agent
runtimes read `SKILL.md` directly; its frontmatter carries the name and the
description used to decide relevance.

Each skill's `references/` files are loaded on demand, so `SKILL.md` stays short
enough to keep in context.

The installer refreshes the process skill's handoff schema and example from the
canonical experimental contracts. The install smoke test verifies the two
runtime homes contain identical bytes.

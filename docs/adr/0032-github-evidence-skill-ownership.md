# ADR 0032: A dedicated proposal-only task skill owns the GitHub evidence surface

- Status: Accepted
- Date: 2026-08-07
- Issue: [#107](https://github.com/CAPHTECH/casegraphen/issues/107)
- Related: ADR 0030 (task skills and process orchestration), ADR 0031 (the
  observation boundary this skill teaches)

## Context

`casegraphen github observe|refresh|project` appeared in exactly one place
across the shipped skills: the auto-generated flag table. Issue #107 measured
that listing is not teaching, and asked which skill should own the surface —
generalize `casegraphen-integrate`, extend `casegraphen-operate`, or add a
dedicated skill — without blurring an existing skill's declared boundary.

A measured baseline (three independent agents driving the surface with only
the flag table; retained under `docs/evals/github-evidence-skill/`) showed
what guidance must carry and what it must not, and ADR 0030 already defines
the extension path: new task skills must add a non-overlapping route and
declare whether their output is read-only, proposal-only, or gated mutation.

## Decision

Add `skills/casegraphen-github-evidence`, a direct task skill whose output is
**proposal-only**: operator-captured GitHub state in, content-addressed
observation records and a compact reviewer projection out, every record
`accepted: false`. Its boundary ends where the store begins — attaching an
observation to a case space is `casegraphen-operate`'s gated `evidence attach`,
and the skill says so instead of teaching it.

The orchestrate routing table gains one row for the surface; the skills README
gains one row; `scripts/skill-conformance.py` gains the skill's directory and
responsibility-contract anchors.

## Rejected alternatives

- **Generalize `casegraphen-integrate`.** Its declared scope and entire body
  are `runtime.node_report.v0` machinery: topology lint, `base_revision_id`,
  `GenericJsonlReconciler` completeness, retry lineage, MCP reconciliation
  tools. The GitHub surface shares the abstract stance (external output
  becomes unreviewed proposals) and not one workflow step — no topology, no
  revision, no reconciler, no store. One description over two disjoint
  workflows is a split wearing a merge's clothes.
- **Extend `casegraphen-operate`.** Its declared boundary is revision-bound
  case-space operation, and its two headline rules — carry the revision, gate
  every durable mutation — are precisely what this surface structurally lacks
  (store-free, revision-free, gate-free by ADR 0031). Teaching it there
  installs the wrong reflexes for these commands and widens the one skill
  ADR 0030 decided to keep whole.

## Consequences

- The trust seam is stated once per side: the new skill stops at
  `accepted: false`; `casegraphen-operate` owns the gated follow-up.
- The skill is subject to the same conformance gate as the others: fenced
  examples are validated against `src/cli_usage.txt`, so the skill cannot
  document a flag the CLI does not declare (the defect class fixed by
  documenting `--strict` on the three `github` usage lines).
- Per the residual method, the skill text is scoped to the measured residual
  and shrinks as the baseline improves; the baseline and ablation evidence is
  retained under `docs/evals/github-evidence-skill/`.

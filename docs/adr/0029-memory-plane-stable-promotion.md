# ADR 0029: Keep the Memory Plane experimental after the first pilot

- Status: Accepted
- Date: 2026-08-06
- Parent decision: [ADR 0028](0028-memory-plane-positioning.md)

## Context

Issue #92 delivers typed source/claim/query/projection/use-report/policy/index
contracts, revision-bound read and proposal surfaces, temporal and authority
governance, conflict visibility, rebuildable indexes, MCP tools, responsibility-
separated Skills, and a bounded CaseGraphen repository pilot.

The retained corpus passes all six safety exit counters and eight adversarial
cases. It does not run a coding agent across enough real sessions to measure
whether projected constraints actually change actions, whether token budgets
are efficient against a baseline, or whether review load is operationally
sustainable. Personal-data deletion is also deliberately undesigned.

## Decision

Keep every Memory Plane contract and surface experimental v0. Do not promote
them into `schemas/casegraphen` or promise backwards compatibility yet.

The next promotion review requires:

1. multi-session coding-agent tasks with retained projections and action traces;
2. measured required-memory recall, stale-use, conflict exposure, source
   sufficiency, and context efficiency against a non-memory baseline;
3. measured constraint-violation and repeated-failure rates;
4. reviewer-volume and latency observations by memory kind/risk class;
5. index deletion/rebuild drills over a non-trivial project corpus;
6. zero violations of the six ADR 0028 safety invariants;
7. a separate privacy/deletion ADR before any personal or sensitive memory.

No automatic acceptance is authorized while the surface remains experimental.

## Consequences

- Experimental v0 contracts may change incompatibly with synchronized Rust,
  schema, example, inventory, CLI, MCP, Skill, and retained-pilot updates.
- The coding-agent pilot may grow, but its memory remains project-scoped.
- A green bounded regression suite is necessary release evidence, not sufficient
  stable-promotion evidence.
- The first pilot's report remains at
  [`../pilots/issue-92/evaluation-report.v0.json`](../pilots/issue-92/evaluation-report.v0.json).

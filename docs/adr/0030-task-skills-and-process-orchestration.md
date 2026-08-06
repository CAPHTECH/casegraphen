# ADR 0030: Separate direct task skills from process orchestration

- Status: Accepted
- Date: 2026-08-06
- Issue: #100

## Context

CaseGraphen ships narrowly constrained skills for design, audit, external-runtime
integration, governed operation, and Memory Plane tasks. They are safe to invoke
directly, but an end-to-end request also needs routing, exact context handoff, and
explicit returns at topology, evidence, revision, worker, and authority seams.

Putting that lifecycle into every task skill would duplicate process decisions
and blur the boundary between proposing work and accepting it. Expanding
`casegraphen-operate` into a universal coordinator would also make a mutation
skill responsible for read-only design and audit routing.

## Decision

Use two explicit layers:

1. direct task skills own one bounded activity and remain independently
   invocable;
2. `casegraphen-orchestrate` owns multi-phase route selection and strict
   `skill.orchestration_handoff.v0` records.

The process skill does not reproduce deterministic graph, completeness, retry,
review, gate, temporal, authority, or acceptance rules. It automatically
continues only between read-only or proposal-only phases with a complete handoff
and no open seam. It returns rather than reviewing, accepting, silently rebasing,
enabling workers, widening scope, or granting authority.

Keep `casegraphen-operate` whole. Its revision, operation-gate, mutation, and
refusal protocol is shared across durable operations; splitting it would create
multiple places that must preserve the same safety contract. Its reference files
remain the internal, on-demand decomposition mechanism.

The handoff contract is experimental, strict, centrally inventoried, and copied
into installed process skills by `install.sh`. The installed schema and example
must remain byte-identical to their canonical repository versions.

## Consequences

- A bounded request should select a direct task skill without invoking the
  process layer.
- An end-to-end request gains an inspectable route and exact handoff inventory,
  but still pauses at non-delegable seams.
- Agent claims cannot close review or authority seams and cannot change the
  `mutation_performed` or `accepted_state_changed` handoff constants.
- New task skills must add a non-overlapping route and declare whether their
  output is read-only, proposal-only, or gated mutation.
- Installer and conformance tests are part of the product boundary because an
  incomplete installed skill can lose the very contract that makes handoff safe.

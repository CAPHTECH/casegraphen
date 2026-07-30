# ADR 0001: Extraction From HigherGraphen And Execution Control Mandate

## Status

Accepted on 2026-07-30. Mirrors HigherGraphen ADR 0003.

## Context

This repository is the standalone home of the HigherGraphen intermediate tool
previously at `CAPHTECH/higher-graphen/tools/casegraphen`. The prior standalone
implementation (a different codebase, the origin of the HigherGraphen thesis)
was renamed to `CAPHTECH/casegraphen-legacy` and serves as reference material
only — no code integration, no compatibility obligation.

Inside HigherGraphen, "Executing scenarios against external systems" was an
explicit non-goal of casegraphen. This repository exists to reverse that
boundary deliberately: the tool takes on execution control (dispatching
accepted work items to workers, validating worker-proposed state transitions
as morphisms, committing verified changes as new revisions) while keeping the
inherited trust invariants intact.

Full investigation and design: `docs/design/independence-and-execution-control.md`.

## Decision

1. Dependency direction: this crate depends ~~only~~ on published
   `higher-graphen-{core, structure, reasoning}` crates. It must not depend on
   `higher-graphen-runtime`; runtime reports may be consumed as evidence input
   JSON only. `higher-graphen-projection` (unused) is dropped;
   `higher-graphen-evidence` is added only when quantitative confidence
   evaluation is needed.

   **Amended by [ADR 0006](0006-dependency-criterion.md).** The word "only"
   formerly imposed a general dependency ban. Other dependencies are now
   admissible only under ADR 0006's measured risk-reduction criterion. The
   prohibition on `higher-graphen-runtime` is unchanged.
2. The execution substrate is the native case space generation (morphism log,
   revisions, replay checksums, close gates). The legacy case-graph generation
   is not carried over. The workflow graph remains a reasoning wire format.
3. Inherited invariants are contract: generated structure stays `unreviewed`
   until an explicit review morphism; inferred evidence never satisfies a hard
   requirement; readiness is derived by replay, never stored; a morphism is
   not applied unless its base revision matches the replayed revision and
   required invariant checks pass or are explicitly waived.
4. Execution trust model: an ExecutionPlan is itself a review target. An
   accepted plan pre-authorizes application of the transition classes it
   declares; deterministic gates (base revision, morphism check,
   pre/postconditions, invariant re-check, evidence origin) guard every step;
   anything outside the plan's declared classes stays unreviewed awaiting
   human review. Worker output is always received untrusted and recorded as
   `source_backed` evidence with content hashes.
5. Side effects are confined to the worker adapter layer (`exec/worker`);
   model, evaluation, and store code stay pure. Effectful workers are disabled
   by default until the Phase 5 security and approval-policy pass.
6. Versioning: publication continues on crates.io as `casegraphen` starting at
   0.8.0 from this repository. New wire contracts get new schema IDs; existing
   strict v1 schemas are never extended in place.

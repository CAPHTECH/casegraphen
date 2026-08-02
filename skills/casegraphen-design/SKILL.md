---
name: casegraphen-design
description: Turn a problem statement into an unreviewed, linted CaseGraphen execution-topology proposal. Use when decomposing governed work into runtime nodes, typed handoffs, resource claims, verification seams, reductions, budgets, or expansion policies before execution. Produces proposal artifacts only; never accepts or mutates a case graph.
---

# Design an execution topology proposal

Design the graph before selecting a runtime. Keep CaseGraphen's acceptance
ledger separate from the runtime topology and from the runtime's later reports.

## Workflow

1. Record the acceptance boundary: case-space id, the exact revision observed,
   goals, evidence requirements, review seams, and non-negotiable world anchors.
   Preserve that revision in every mapping proposal; never replace it with the
   current revision implicitly.
2. Separate CaseGraphen cells from runtime micro-nodes. Create a cell mapping
   only when completion affects readiness or requires governed evidence. Keep
   retries, tool calls, token streams, and other runtime detail out of the case
   graph.
3. Draft `execution.topology.json` against
   `casegraphen.experimental.execution.topology.v0`. Define every node's typed
   inputs and outputs, side effects, resources, executor class, idempotency,
   delivery mode, policies, and provenance. Read
   [contracts-and-outputs.md](references/contracts-and-outputs.md) before
   authoring artifacts.
4. Classify each necessary dependency as data, control, evidence,
   review/authority, resource exclusion, or temporal. Supply its blocking
   predicate, dependency witness, and removal counterexample. Model shared
   resources explicitly; do not infer independence from different work-cell
   ids.
5. Select fan-out, reduction, synthesis, barrier, and verification seams. For
   large collections, read [patterns.md](references/patterns.md). For verifier,
   budget, expansion, or runtime metadata boundaries, read
   [policies-and-trust.md](references/policies-and-trust.md).
6. Emit the proposal artifacts into a caller-selected output directory. Do not
   invoke any CaseGraphen mutation, review, acceptance, plan-acceptance, or
   worker command.
7. Invoke the shipped deterministic linter; never reproduce its checks in
   prose, a script, or agent judgment:

   ```sh
   casegraphen graph lint --input execution.topology.json --format json \
     --output graph.analysis.report.json
   ```

8. Read the linter's classifications and locations. Revise the proposal for
   errors, retain deterministic warnings and heuristics in the report, and run
   the linter again. Do not relabel a heuristic as deterministic or suppress a
   finding by editing the report.
9. Return paths, schema stability, the observed base revision, remaining lint
   findings, and which artifacts still require human or policy review.

## Required outputs

- `execution.topology.json` — experimental v0 proposal.
- `graph.analysis.report.json` — exact output from `graph lint`.
- `genesis.mapping.proposal.md` — mapping proposal, unless a separately
  validated stable `native.case.space` genesis was explicitly requested.
- `execution-plan.mapping.proposal.md` — mapping proposal, unless a separately
  validated stable execution plan was explicitly requested.
- `verification.policy.json` — only when verification policy is used; label it
  design metadata until a contract governs it.
- `runtime.deployment.json` — only when deployment choices are used; label it
  runtime-owned and unreviewed.

## Non-negotiable boundary

- A topology and lint report are proposals, never accepted state.
- Never call `morphism apply`, `plan accept`, `review accept`, `cell transition`,
  `evidence attach`, `packet apply`, `run`, or `operate` from this Skill.
- Never run an LLM through the shell worker.
- Never invent scheduling, retry, resource-reservation, or acceptance behavior.
- Never treat runtime-reported actor, model, context, cost, or freshness as an
  accepted fact.

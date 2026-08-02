---
name: casegraphen-audit
description: Audit a CaseGraphen execution topology and, when canonical runtime completeness data is available, compare planned and reported behavior. Use for dependency, critical-path, barrier, resource, fan-in, verification, authority, retry, missing-report, schema, latency, cost, allocation, or convergence investigations. Keeps deterministic violations, runtime observations, runtime-declared metadata, and review-required inferences separate and never promotes runtime claims.
---

# Audit graph shape and runtime reports

Audit without changing the topology, case ledger, runtime reports, or review
state. Treat static structure, runtime declarations, and accepted evidence as
different boundaries.

## Workflow

1. Record the topology id/content hash, case-space id, observed revision, report
   inventory, independently observed artifact inventory, and unavailable
   evidence. Never silently rebase an audit.
2. Run the shipped static analyzer. Do not recreate its graph algorithms or
   thresholds:

   ```sh
   casegraphen graph lint --input execution.topology.json --format json \
     --output graph.analysis.report.json
   ```

3. Interpret every static finding using its emitted `classification`; read
   [static-audit.md](references/static-audit.md). A heuristic remains an
   inference requiring review.
4. If runtime reports exist, validate them as
   `casegraphen.experimental.runtime.node_report.v0`. Obtain completeness only
   from a host integration that calls the library's
   `reconcile_runtime_reports` with an independently content-addressed
   `RuntimeGraphExpectation`, the reports, and the observed artifact ids. Do
   not count reports or reconstruct retry lineage in the Skill. If canonical
   completeness is unavailable, stop the run-audit portion and report the
   missing integration.
5. Read [run-audit.md](references/run-audit.md). Compare the plan and reports
   only after the canonical completeness result is present. A runtime status or
   a 199/200 report set never proves completion.
6. Classify each statement with exactly one evidence class from
   [reporting-boundary.md](references/reporting-boundary.md). Preserve source
   ids and distinguish absence of evidence from evidence of absence.
7. Emit `graph.audit.report.md` containing scope, inputs/hashes, static report
   path, canonical completeness as received, categorized findings, unresolved
   questions, and next review/observation operations. Report corrections as
   proposals only.

## Non-negotiable boundary

- Never invoke a mutation, review, evidence, transition, worker, `run`, or
  `operate` command.
- Never promote runtime status, identity, model, context, timestamps, cost,
  resources, or verifier declarations to accepted facts.
- Never implement a second completeness, retry-lineage, schema-match, graph
  join, or artifact-accounting rule.
- Never call a heuristic a violation or infer verifier false positives without
  an independently governed world anchor.
- Never edit the linter or completeness result to make the run appear complete.

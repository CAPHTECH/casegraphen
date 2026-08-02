# Planned-versus-reported audit

## Required inputs

- the exact topology id and content hash used to build
  `RuntimeGraphExpectation`;
- validated `runtime.node_report.v0` values;
- independently observed artifact ids;
- the unmodified `RuntimeCompleteness` returned by
  `casegraphen::runtime_protocol::reconcile_runtime_reports`;
- optional planned duration, budget, resource, expansion, and verification
  policies.

In a repository checkout, validate report shape against
`schemas/experimental/runtime.node_report.schema.json`. A user-level
installation includes a byte-for-byte copy at
`references/runtime.node_report.schema.json`. Semantic validation and
completeness still belong to `casegraphen::runtime_protocol`, not the JSON
Schema or this Skill.

Do not derive completeness locally. The canonical result already covers graph
joins, expected/missing nodes, failures, duplicate/retry lineage, output-schema
agreement, and artifact accounting. Quote its counters and finding codes. In
particular, `missing_report_count > 0` or `complete == false` is incomplete even
when every received report claims success.

## Comparisons outside completeness

Keep these as observations or inferences, not acceptance decisions:

- predicted versus runtime-declared start/finish latency and cost;
- planned claims versus runtime-declared resource allocations;
- repeated retry patterns or expansion rounds suggesting non-convergence;
- verifier disagreement, correlation, or possible false positives.

Name the arithmetic, units, currency, time basis, and missing samples. Do not
aggregate incomparable currencies or trust runtime clocks without an anchor.
Identity, model, context, cost, allocation, freshness, and runtime timestamps
are declarations from an untrusted boundary. A verifier false-positive claim
requires an independent world anchor; otherwise label it an inference and name
the evidence needed.

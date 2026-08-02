# Static topology audit

Use only the exact JSON emitted by `casegraphen graph lint`. Preserve its
topology content hash, metrics, finding code, location, severity,
classification, detail, and suggested next operation.

Review these lenses without adding a parallel decision rule:

- dependencies: deterministic cycles/reachability facts and heuristic false or
  missing-edge candidates;
- execution shape: critical path, theoretical width, barriers, and fan-in or
  context pressure;
- safety: conflicting resources and worktree integration risk;
- governance: verification-policy visibility, possible verifier correlation,
  missing anchors, and authority concentration;
- dynamic work: termination and budget visibility.

The linter can prove only what its typed contract exposes. A missing dependency,
semantic merge conflict, context collapse, verifier correlation, or absent
world anchor may remain an inference. State the required counterexample or
external observation instead of upgrading it to a deterministic violation.

For edge-removal review, ask whether removal changes permitted execution,
acceptability, resource safety, or auditability. This question guides review;
it does not replace the linter finding.

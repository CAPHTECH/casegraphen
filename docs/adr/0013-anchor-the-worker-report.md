# ADR 0013: The Execution Trace Anchors The Worker Report And Both Raw Streams

## Status

Accepted on 2026-08-01. Decides the contract question in issue #8 before it is
implemented, because it changes a shipped record shape.

## Context

`execution.trace.json` is anchored: `trace_content_hash` goes into the log and
`verify_recorded_trace_anchors` re-hashes the file on read, so tampering with
the trace is caught. The trace carries `worker_report_id` — an id, not a hash.
The evidence cell carries the stdout hash and the exit status.

Nothing durable carries the **stderr** hash, `timed_out`,
`descendants_may_survive`, `byte_len`, or the truncation and incompleteness
flags. Those live only in `runs/<trace>/worker.report.json`, and nothing reads
that file back. The fourth adversarial round rewrote a completed step's report —
flipping every output flag, zeroing the hashes, changing
`binding_content_hash` — and overwrote the stderr file; `space validate`,
`space rebuild`, and `space replay` all stayed green and a later `run --step`
proceeded normally.

`docs/security/worker-execution-policy.md` §2.6 states the audit chain as
"trace → worker report + raw output hashes → anchored log entries". The middle
link is not anchored, so the chain has a gap exactly where the forensic detail
lives. This is narrower and quieter than residual risk 2: rewriting the log has
to survive the hash chain, rewriting the report has to survive nothing.

## Decision

Fold three SHA-256 values into `ExecutionTrace` before it is hashed and
anchored, so the trace anchor transitively covers the report and both raw
streams:

- `worker_report_content_hash` — the bytes of the `worker.report.json` the
  dispatch wrote,
- `stdout_content_hash` and `stderr_content_hash` — the same hashes the worker
  report records, computed over the full stream (including the part dropped by
  the 4 MiB cap, as they already are).

**A new schema id, `highergraphen.case.workflow.execution_trace.v2`**, not
optional fields on `.v1`. These are required and verified: a `.v1` trace with
optional hashes would mean "sometimes anchored", and the whole value of the
change is that a reader can rely on them without checking which kind of trace
they hold. Per the `contract-change` decision table, a required field on an
existing strict record is a new `$id`, and this repository does not carry
backward compatibility. Traces written by an earlier version stop parsing;
that is the intended direction, because such a trace does not carry what the
policy will claim it carries.

**Verification runs where the anchor is already verified.** The reader that
re-hashes the trace additionally re-hashes `worker.report.json` and both
streams and compares them against the trace's values. One verification site,
not a second copy of the rule beside the existing one.

## Consequences

- `schemas/casegraphen/execution.trace.schema.json` gets the `.v2` `$id` and
  the three required fields, with its example updated in the same change, plus
  the Rust constant, `tests/schema_ids.rs`, and
  `report-schema-aliases.json` if the trace id appears there.
- A store holding `.v1` traces fails to read them. Fixtures move to the
  stricter shape rather than the check being relaxed.
- Policy §2.6's audit chain becomes true as stated: the anchored trace covers
  the report and both raw streams. Residual risk 2 is unchanged — a writer who
  can rewrite the log tail can still rewrite everything downstream of it — but
  the quieter capability of rewriting only what a worker was recorded as having
  done is gone.
- The dispatch path is touched, so the `adversarial-execution-reviewer` runs
  and its findings are reproduced before acceptance, per the working
  agreements.

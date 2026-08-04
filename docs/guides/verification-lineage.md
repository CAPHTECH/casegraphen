# Ledger-derived verification lineage

Verification-policy reconciliation is a read-only assessment. It never accepts
evidence, and caller-declared actor, capability, disposition, or quorum values
never satisfy ledger requirements.

## Supported native CLI workflow

The shipped shell-worker path supports this authority chain:

```text
accepted plan
→ casegraphen run --step
→ worker.report.json + execution.trace.json + stdout/stderr
→ tool-minted execution_trace_anchor
→ worker evidence claim
→ casegraphen review accept|reject by a distinct authorized actor
→ derive_native_cli_run_producer_proof
→ derive_native_cli_review_verifier_proof
→ reconcile_verification_policy
```

`derive_native_cli_run_producer_proof` takes a replayed `CaseSpace` and the
exact retained report, trace, stdout, and stderr bytes. It derives the actor,
capabilities, step, attempt, trace anchor, and evidence-attachment relationship
from those records. A caller does not translate a `WorkerReport` into a
synthetic runtime-node report.

The normal review command is a judgment, not another worker execution.
`derive_native_cli_review_verifier_proof` therefore derives verifier authority
from the canonical, content-bound review morphism and its operation gate. It
does not ask an integrator to fabricate an `ExecutionTrace` for a human review.

The shared producer/verifier subject revision is the run trace's
`base_revision_id`. The later review revision supplies verifier authority but
does not replace that subject. Reconciliation always uses the current replay,
so a later reopen/reject, an invalidated capability grant, a missing claim, or
a missing authority morphism invalidates retained proofs.

## Review seam

The worker evidence remains unreviewed until the independent
`casegraphen review` operation. Deriving a producer proof does not cross that
seam. Even when every verification-policy requirement is satisfied, the
result is not a CaseGraphen evidence-acceptance mutation.

Generic external runtimes continue to use `derive_ledger_producer_proof` and
`derive_ledger_verifier_proof` with exact `RuntimeNodeReport` and execution
trace bytes. Do not mix the generic runtime contract with the native
`WorkerReport` contract.

## Operational MCP path

Call `reconcile_verification_lineage` to use the native workflow without
custom Rust. The configured artifact root must contain the retained run files;
all paths are normal relative paths, and symlinks or paths resolving outside
that root are refused.

```json
{
  "request_id": "request:verification-lineage-1",
  "idempotency_key": "verification-lineage:claim-1:revision-9",
  "base_revision_id": "revision:9",
  "payload": {
    "verification_lineage": {
      "case_space_id": "case-space:example",
      "claim_cell_id": "evidence:worker-output",
      "policy": { "schema": "casegraphen.experimental.verification_policy.v0" },
      "producer_files": {
        "worker_report_path": "runs/run-1/worker.report.json",
        "execution_trace_path": "runs/run-1/execution.trace.json",
        "stdout_path": "runs/run-1/stdout",
        "stderr_path": "runs/run-1/stderr"
      },
      "review_morphism_ids": ["morphism:review-1"],
      "anchors": [
        { "kind": "execution_trace", "anchor_id": "test-execution" }
      ]
    }
  }
}
```

The abbreviated policy above only illustrates placement; submit a complete
`verification_policy.v0` document. Review IDs and anchor IDs must be unique.
The host replays `base_revision_id`, reads the exact bytes, and invokes the
canonical opaque constructors. Its response deliberately contains
`proofs_serialized: false`, `read_only: true`, `mutation_performed: false`, and
`accepted: false`. A policy may be satisfied, but only a separate canonical
review/mutation path can change evidence acceptance.

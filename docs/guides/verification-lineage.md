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

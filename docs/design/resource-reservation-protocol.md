# Experimental resource reservation protocol

Issue #50 adds a deterministic external-runtime boundary between topology
claims and runtime allocation. It does not add a scheduler, lock service, git
command runner, secret store, or CaseGraphen acceptance rule.

## Four separate facts

```text
execution topology
  -> resource.declaration.v0       (what a node says it needs)
  -> resource.reservation.v0       (what an allocator grants to one attempt)
  -> runtime.resource_allocation.v0 (what the runtime says it actually used)
  -> resource.reconciliation.v0    (deterministic comparison)
```

The records join by declaration, reservation, attempt, graph, and resource
identities. A runtime allocation retains an untrusted marker. Reconciliation
can reject a mismatch; it cannot accept evidence or prove that a runtime's
declaration reflects the world.

Resource identifiers and authorization scopes are namespaced ids such as
`file:src/lib.rs`, `git-branch:main`, `network:git-host`, and
`secret:deployment-signing`. A `secret_scope` names an authorization only. A
secret value, environment assignment, credential, or token must never enter any
protocol record.

## Reservation rules

- Read/read access to the same resource is compatible.
- Write or exclusive access conflicts with every overlapping mode except there
  is no read exception for either writer.
- Every named rate-limit group consumes the declared unit count and requires an
  explicit `resource.rate_limit_capacity.v0` record.
- A grant must exactly reproduce its topology declaration in v0; an allocator
  cannot silently widen scopes or omit a claim.
- Release and supersede are
  `resource.reservation_disposition.v0` assertions joined to both reservation
  and attempt. Supersede must name a distinct successor reservation.
- `granted_at` is audit metadata only. No duration, expiry, mtime, timeout, or
  clock observation releases a reservation. Crash recovery requires an
  explicit externally justified assertion, following ADR 0017.

The pure `grant_reservation` function evaluates a proposed grant against a
caller-supplied set of reservations, disposition assertions, and capacities.
The caller remains responsible for serializing competing grants atomically.
Calling the function concurrently against stale snapshots is not an allocation
service and does not enforce mutual exclusion.

## Reference git worktree record

`git.worktree_record.v0` records an isolated worktree/branch created from an
explicit base commit, its reservation and attempt, path identity, resulting
commit, clean/uncommitted state, and unexpected writes. Cleanup must be both
recoverable and explicitly asserted. The reference fixture contains records
for two isolated code-changing attempts and two distinct commits; tests do not
create, remove, prune, or otherwise mutate real worktrees.

An adapter implementing the record should:

1. reserve the shared main branch/workspace identity before mutation;
2. create a distinct worktree and branch from the recorded base SHA;
3. report the isolated worktree allocation under the same attempt;
4. inspect clean state and unexpected paths before recording the result SHA;
5. retain the worktree on failure or crash until an operator asserts recoverable
   cleanup or supersession.

The protocol deliberately records cleanup intent rather than performing it.
This keeps destructive filesystem behavior outside the acceptance kernel and
outside deterministic tests.

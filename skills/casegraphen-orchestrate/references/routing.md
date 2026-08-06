# Task-skill routing

Direct invocation remains the default for a bounded task. The process skill is
for requests spanning phases or needing route selection.

| Intent | Direct task skill | Output boundary |
|---|---|---|
| Design/decompose governed runtime work | `casegraphen-design` | unreviewed topology/policy proposals |
| Audit topology or canonical runtime observations | `casegraphen-audit` | read-only findings and unresolved review judgments |
| Reconcile generic external-runtime JSONL | `casegraphen-integrate` | unreviewed evidence/morphism proposals |
| Mutate or advance the acceptance ledger | `casegraphen-operate` | gated mutation result or explicit refusal |
| Read accepted project memory | `casegraphen-memory-query` | read-only revision-bound projection |
| Curate project memory | `casegraphen-memory-curate` | source-bound unreviewed proposals |
| Audit governed memory | `casegraphen-memory-audit` | read-only trust/temporal/replay findings |

Do not route by whichever skill can make progress. Route by ownership of the
next boundary. In particular, the process skill cannot substitute for
`casegraphen-operate` at a mutation seam or for an independent reviewer at a
review seam.

## Continuation rule

Continue automatically only when all are true:

1. the completed phase was read-only or proposal-only;
2. the next phase is read-only or proposal-only;
3. the exact handoff validates;
4. no review, authority, worker-enablement, credential, scope, or stale-revision
   seam is open;
5. no required evidence is unresolved.

Otherwise emit a handoff with `return_required: true` and stop.

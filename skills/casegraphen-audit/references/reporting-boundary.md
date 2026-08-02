# Audit evidence classes

Assign exactly one class to every finding or statement.

| Class | Use for | Required wording discipline |
|---|---|---|
| `deterministic_violation` | Contract/linter deterministic violations and canonical reconciliation findings such as missing reports or schema mismatch | Cite the rule owner, code, location/node/attempt, and exact input hash |
| `observation` | Receipt/presence of a report, emitted counters, independently observed artifacts, and literal runtime status/timestamps/artifact fields | Say “reported” for runtime-sourced values; do not claim the world matched them |
| `runtime_declared` | Runtime identity/version, adapter, model, context, token/cost, allocation, worktree, commit, and freshness declarations | Prefix with “runtime-declared, untrusted” and retain the trust marker |
| `inference` | False/missing semantic edges, causal latency claims, correlation, false positives, convergence, or policy adequacy requiring review | State evidence, counterevidence, uncertainty, and the review/anchor needed |

Do not collapse classes into a single severity. Severity expresses impact;
class expresses epistemic source. A canonical finding can establish a protocol
violation but cannot accept evidence, prove actor independence, or transition a
case.

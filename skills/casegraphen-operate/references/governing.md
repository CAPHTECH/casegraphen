# Governing an agent runtime's graph

Use this when the work is executed by an agent runtime (LangGraph-class or any
multi-agent harness) and CaseGraphen is the record of what was accepted.
`$STORE`, `$CS`, `$GATE`, and `cur()` are from SKILL.md.

The division of labour is fixed by
[ADR 0002](https://github.com/CAPHTECH/casegraphen/blob/main/docs/adr/0002-graph-engineering-positioning.md):
**the runtime executes; CaseGraphen records acceptance.** It is the system of
record for the accepted topology and its authority, the pre-authorized
transition classes, the trust status of every node output, and a replayable
account of all of it. It is not a scheduler, a message bus, or a model caller,
and it will not become one.

Two consequences you must design around:

- **Do not run LLM nodes through `run --step`.** The only worker kind is
  `shell`; a model call means a script carrying an API key through
  `env_allowlist`, which is residual risk 3 of the worker execution policy. Keep
  `run --step` for deterministic gate nodes (a checker, a linter, a test
  command) and let the runtime call models.
- **There is no fan-out.** One invocation advances at most one item, under a
  per-case lock. The frontier tells you which nodes are parallelizable; the
  runtime is what runs them in parallel.

## Granularity: which runtime nodes become cells

A node of the runtime's work graph earns a case cell only when **its completion
changes what other work may proceed**, or when **it requires evidence a human or
a policy must be able to check**. Everything else belongs to the runtime's own
trace.

| Runtime thing | In CaseGraphen |
|---|---|
| A phase whose completion unblocks other phases | a `work` cell plus its `depends_on` / `requires_evidence` relations |
| A human checkpoint | the review morphism that records the decision, not a cell of its own |
| An individual LLM call, retry, tool call, token stream | nothing — runtime trace only |
| A whole agent run that produced a reviewable artifact | at most **one** evidence cell, content-hashed, pointing at the runtime's artifact |
| The runtime's own execution trace for a node | one evidence cell (or a URI in that cell's metadata) attached to the governed node |

This is a semantic rule, not a load budget. A cell asserts "someone must be able
to check this", so a cell per model call dilutes the ledger into noise even
though the tool can now carry it: derivation is linear — about 0.02 s at 1,000
cells, 0.20 s at 10,000, 1.92 s at 100,000. The cost that does grow is storage,
because every revision writes a full snapshot (about 287 MB for a 100,000-cell
space). If you find yourself minting a cell per model call, the granularity is
wrong even though it would run.

## Mapping the org graph onto authority

The runtime's org graph — which agent owns what, which tools it may reach — maps
onto capability cells, which enter **only** in a native genesis:

- one `custom:capability` cell per mandate, not per agent (an agent that reviews
  and an agent that dispatches must not share one);
- `metadata.actor_ids` lists the runtime identities that hold it, using the same
  identifiers the runtime puts in its own traces so the two records join;
- there is no CLI path to grant, amend, or revoke afterwards. Decide the mandates
  before lifting; changing them means lifting a new case space.

A space produced by `lift workflow` contains no capability cells, so it can
record nothing: it is an analysis space. To govern execution, author a native
genesis (see `references/authoring.md`) that declares the capabilities, and
carry the graph's structure into it.

## Taking a runtime report as evidence

A runtime report is untrusted input, exactly like a worker report. Never a build
dependency, never a trust source — the mapping is fixed:

| Runtime artifact | Enters as |
|---|---|
| Output of an executed process | evidence, `evidence_boundary: worker_output`, `unreviewed` |
| Model reasoning, a summary, a judgment | evidence, `evidence_boundary: inferred`, `unreviewed` |
| The runtime's claim that a node "succeeded" | a finding, not an achievement. Hard requirements are satisfied only by source-backed or promoted evidence |
| An approval clicked in the runtime's UI | **not trusted.** That is a caller-declared trust value. Re-express it as a gated `review accept` naming reviewer, reason, and evidence |
| Cost and latency figures | recordable inside the evidence cell's `metadata`; CaseGraphen does not enforce budgets |

The loop per governed node, once the runtime reports:

```sh
# 1. record what the runtime produced, untrusted
casegraphen evidence attach --store "$STORE" --case-space-id "$CS" \
  --base-revision-id "$(cur)" --input node-output.evidence.json \
  --satisfies <requirement-id> $GATE --format json

# 2. a human (or a deterministic check) promotes it — this is the acceptance
casegraphen review accept --store "$STORE" --case-space-id "$CS" \
  --target-id <requirement-id> --reviewer-id <id> --reason "<what was verified>" \
  --base-revision-id "$(cur)" --evidence-id <attached id> $GATE --format json

# 3. only now does the node's state change
casegraphen cell transition --store "$STORE" --case-space-id "$CS" \
  --base-revision-id "$(cur)" --cell-id <node> --to resolved \
  --reason "<why>" $GATE --format json

# 4. what may proceed next is derived, not decided by you
casegraphen space frontier --store "$STORE" --case-space-id "$CS" --format json
```

Attaching does not promote, and promoting does not edit the cell — step 2 lives
in the log, which is what makes "who accepted this, on what evidence" answerable
later. See `references/mutating.md`.

## What this buys, and what it does not

Answerable afterwards, per node: who accepted it, under which capability,
against which content hash, backed by which evidence at which trust boundary,
and whether the whole history folds from empty to the current state
(`space validate`). "The graph did it" is not a possible answer.

Not provided: parallel dispatch, typed handoff of one node's output into
another's input, model-identity pinning (a binding pins a script's hash, not a
model, version, or prompt), cost enforcement, and any memory of past
misjudgments. Do not simulate these with case cells; they belong to the runtime.

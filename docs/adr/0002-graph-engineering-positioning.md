# ADR 0002: Positioning Within Graph Engineering

## Status

Accepted on 2026-07-30.

## Context

In mid-July 2026 the agent-engineering discourse moved from loops to graphs.
The progression the field itself narrates is prompt → context → harness → loop
→ graph engineering; the compressed claim is "loops are subroutines, graphs are
programs": a loop makes one agent's behaviour programmable, a graph makes a
multi-agent *organization* programmable. The vocabulary distinguishes a
long-lived **org graph** (agents, mandates, message paths) from a runtime
**work graph** (task nodes that spawn, merge, and disappear as evidence
arrives), and it explicitly separates itself from knowledge graphs: a knowledge
graph structures what a system knows, graph engineering structures who the
system is and what it may do. (References at the end.)

The same discourse names its own open problems. The ones that matter for this
decision:

- **Attribution loss** — "the graph did it" is not an audit answer.
- **Determinism and replay** — not discussed at all in the surveyed material.
- **Trust of node output** — no mechanism distinguishes "an agent said so" from
  "this is an accepted fact"; cross-agent prompt injection is acknowledged
  without a containment story.
- **Human approval** — appears as UI checkpoints, not as a verifiable record of
  who authorized what, under which authority, against which content.
- **Failure propagation** — node failures leaving the work graph in an
  inconsistent, unaccounted state.
- **Handoff** — converting one node's output into another node's input is
  named unsolved.
- **Judgment precision** — the critique that wiring cannot compensate for weak
  branch-point judgment: an elaborate graph is "a device for moving fast in the
  wrong direction".

Governance does exist in the field, but at **call granularity**: gateway-style
identity, per-node tool policy, cost/latency telemetry. What has no owner is
governance at **acceptance granularity**: which topology was reviewed and
accepted, which transitions were pre-authorized, which outputs became facts,
and whether any of it can be replayed and audited later.

That is what this tool already enforces, and each claim below was demonstrated
against the real binary (the section numbers refer to
`docs/guides/release-decision-walkthrough.md`):

- every durable mutation carries actor + capability + scope + boundary in a
  hash-chained, fold-from-empty replayable log (§13);
- plan acceptance is content-addressed pre-authorization; an edited pinned
  worker never runs (§11);
- node output enters as `unreviewed` evidence with a typed boundary and cannot
  satisfy a hard requirement without promotion through a gated review (§6, §8);
- the work graph is mutable mid-execution and readiness re-derives (§7);
- a failed node is a domain finding with evidence and an anchored trace, never
  silent inconsistency (§10);
- injected or fabricated agent output is not prevented, but its *consequence*
  is contained: it enters untrusted and cannot promote itself
  (never-trust-caller-declared-trust, ADR 0001).

Equally, this tool lacks what the field defines as the execution harness's job.
`run --step` advances exactly one item under a per-case lock; there is no
fan-out. Step-to-step dataflow and typed handoff do not exist; the
execution-plan contract has no field that supplies one step's output as another
step's input. Cost is not tracked; model identity is not part of a binding's
content address; the only worker kind is `shell`.

This ADR originally also recorded a size limit: readiness derivation measured
O(n²) (0.23 s at 1,000 cells, 0.84 s at 2,000, 3.66 s at 4,000, 32 s at
10,000, best of three release-build runs), and decision 2 below drew its
boundary partly from that number. The cause was per-cell linear rescans of the
relation list and of the morphism log; indexing them once per evaluation made
derivation linear. The same measurement now reads 0.02 s at 1,000 cells, 0.07 s
at 4,000, 0.20 s at 10,000, and 1.92 s at 100,000 — a 162× improvement at
10,000 cells, with byte-identical output on four real stores.

What remained at the time of this decision was a storage cost: every revision
wrote a full snapshot, so a 100,000-cell space occupied about 287 MB and each
edit added another snapshot. ADR 0005 amends that implementation: snapshots are
periodic, intervening revisions replay from the nearest snapshot, and a
constant-size log-head file independently anchors the unsnapshotted tail.

## Decision

1. **CaseGraphen is the acceptance ledger of a graph-engineered system, not
   its runtime.** It is the system of record for: the accepted topology and
   its authority (who reviewed it, against which content hash), the
   pre-authorized transition classes, the trust status of every node output,
   and a replayable account of all of it. Gateways govern *access at call
   time*; CaseGraphen governs *acceptance at state-change time*. We do not
   compete with LangGraph-class runtimes on execution, and we do not accept
   execution-harness features (schedulers, message buses, retry engines) into
   this crate to chase that role.

2. **Granularity rule.** A node of the runtime's work graph earns a case cell
   only when its completion changes what other work may proceed, or when it
   requires evidence a human or policy must be able to check. Individual LLM
   calls, retries, tool invocations, and streaming belong to the runtime's own
   trace; at most, that trace enters as *one* evidence cell (content-hashed
   artifact or URI) attached to the governed node.

   This rule is semantic, not a performance budget. It survived the linearity
   fix above unchanged in substance and lost its numeric ceiling: a cell asserts
   that someone must be able to check this, so a cell per model call is noise
   that dilutes the ledger rather than a load problem. Derivation is now
   comfortable into the tens of thousands of cells, so a domain that genuinely
   has that many *decisions* is in range. The constraint recorded here was the
   per-revision snapshot cost, not evaluation time; ADR 0005 supersedes that
   storage-policy detail.

3. **Integration contract: runtime reports enter as evidence input JSON.**
   This extends the rule ADR 0001 already sets for `higher-graphen-runtime` to
   agent runtimes in general: no build dependency on any runtime, consumption
   of its reports as untrusted input only. The trust mapping is fixed:

   | Runtime artifact | Enters CaseGraphen as |
   |---|---|
   | Node output produced by an executed process | evidence, boundary `worker_output`, `unreviewed` |
   | Node output that is model reasoning | evidence, boundary `inferred`, `unreviewed` |
   | Runtime's claim that a node "succeeded" | a domain finding; success is not goal-achievement, hard requirements are satisfied only by promoted or source-backed evidence |
   | Approval clicked in a runtime's UI | **not trusted.** A caller-declared approval is a caller-declared trust value; the approval must be re-expressed as a gated `review accept` naming reviewer, reason, and evidence |

4. **The existing worker path stays for deterministic gate nodes** (the
   walkthrough's schema gate is the canonical shape: a pinned script, cleared
   environment, hashed output). It is not the path for LLM nodes: `shell` is
   the only worker kind, an API key would ride `env_allowlist`, and that is
   exactly residual risk 3 of the worker execution policy. Running models is
   the runtime's job under this decision.

## Non-goals

- ~~**No parallel dispatcher (`run --frontier`).** Fan-out conflicts with the
  single append-only revision chain; resolving that (batch morphism per round,
  or optimistic append with retry) is its own ADR if we ever take it.~~
  **Superseded by [ADR 0004](0004-frontier-dispatch.md).** The stated conflict
  was not real: the chain constrains appends, not execution, so workers run
  concurrently while results append serially and the log keeps its sequential
  shape. Recorded here rather than deleted because the mistake is the
  instructive part.
- **No message bus, daemon, or scheduler.** Reaffirms the design doc's
  explicit exclusion; the runtime owns liveness.
- **No cost ledger.** Cost spiral is a real named problem, but enforcement
  belongs to the runtime/gateway. A runtime report that carries cost figures
  can be recorded today inside evidence `metadata` with no contract change —
  record, don't enforce.
- **No LLM worker kind, and no model-identity pinning.** This bullet used to
  defer the question "until an LLM worker kind is actually proposed". It was
  proposed — issue #26 asked whether registering `codex` or `claude` as the
  execution subject would stop an agent working outside the ledger and
  hand-writing evidence about it afterwards — and it is **refused**. Recorded
  here so the next asker gets the argument rather than re-deriving it.

  Registering an agent CLI as a `shell` worker works mechanically (`command`
  takes an absolute path) and breaks three things:

  1. **The content address keeps its form and loses its meaning.**
     `command_content_hash` is the sha256 of the command file's bytes
     (`src/exec/binding.rs`). For a pinned script that reads "the same program
     runs, so the same behaviour follows". For an agent CLI the hash still pins
     the binary while the behaviour differs on every invocation. Nothing
     refuses; the promise evaporates while the field still looks satisfied,
     which is worse than not having the field, because a reviewer reading a
     hashed binding believes something.
  2. **Credentials ride `env_allowlist`.** An API key is exactly residual
     risk 3 of the worker execution policy — "a reviewer can still approve
     another secret-bearing variable" — on every agent binding rather than as
     an exception.
  3. **The trust boundary would say `worker_output` where `inferred` is true.**
     Decision 3's table maps *node output produced by an executed process* to
     `worker_output` and *node output that is model reasoning* to `inferred`;
     an agent CLI run as a shell worker produces the second and would be
     labelled the first. To be precise about the severity rather than
     overstate it: `src/evidence_trust.rs` handles `WorkerOutput` and
     `Inferred` **identically** — both require an accepted review before
     satisfying anything — so this opens no authority hole. It is an
     audit-truth defect, not a weakened gate, and the refusal does not rest on
     inflating it.

  What decided it is the baseline, not those costs. `evidence attach
  --artifact` (issue #18) makes an agent's diff, log, and test output
  content-addressed cells **the tool hashed**, cited by a claim that review
  lands on. "The agent works invisibly and self-reports" is already fixed. The
  only delta a worker kind adds is that the tool saw the process start and
  anchored its trace (ADR 0013) rather than hashing files handed over
  afterwards — and that delta does not pay for a wire-contract change, the
  security-policy design review this ADR requires for any `worker_kind` beyond
  `shell`, and a content address advertising reproducibility it cannot deliver.

  Registration would also buy less than the operator report that prompted it
  asked for. Of the five things driven by hand — plan, binding, worker, retry,
  evidence acceptance — it moves exactly one. Retry is an explicit act by
  design (ADR 0004), review is the actor seam and cannot be delegated to the
  agent that produced the claim (ADR 0015), and the loop is ADR 0016's subject.
  The halts were never about who typed the command.

  **What would reopen this:** a design that drops the pin instead of faking it
  — pinning only what is observable (the invoked binary, prompt text,
  arguments, environment allowlist), declaring in the binding contract that
  behaviour is *not* reproducible from the content address, and entering output
  at `inferred`. Model identity itself stays out regardless: the tool does not
  make the API call, so a declared `model:` is a caller-declared trust value,
  and putting one inside a content address would make the address carry exactly
  the kind of claim the rest of this crate refuses.
- **No typed step handoff.** The execution-plan contract intentionally has no
  step-input field. Any future decision must introduce and implement typed
  handoff through the contract-change process rather than advertise a
  half-alive hook.
- **No memory of past misjudgments.** The judgment-precision critique is only
  half-answered here: branch decisions must be evidence-backed and a wrong one
  surfaces as an obstruction rather than passing silently, but nothing
  accumulates "this judgment failed before". Out of scope for a ledger.

## Consequences

- The next implementation step, when a concrete runtime is chosen, is a small
  adapter: runtime report → evidence-cell JSON → gated `evidence attach` →
  gated `review accept` → transition. No CaseGraphen code change is required
  to start; the adapter is external by design.
- The positioning claim in this ADR is falsifiable the same way the security
  policy is: every "already enforced" bullet cites a walkthrough section that
  reproduces it against the shipped binary. If a refactor breaks one, this
  ADR — not just the walkthrough — is stale and must be revisited.

## References

Surveyed 2026-07-30. External content; claims above about "the field" are
theirs, claims about this tool are ours.

- Graph engineering origin and org/work-graph vocabulary:
  <https://explainx.ai/blog/graph-engineering-ai-agents-multi-agent-organizations-2026>
- Call-granularity governance and named failure modes (attribution loss, cost
  spiral, injection):
  <https://www.truefoundry.com/blog/graph-engineering-enterprise-guide>
- Runtime primitives that matter (cycles, dynamic fan-out, mixed determinism):
  <https://www.langchain.com/blog/3-years-of-graph-engineering-with-langgraph>
- Judgment-precision critique:
  <https://zenn.dev/jodycraft/articles/dfe38e73e1ef8a>

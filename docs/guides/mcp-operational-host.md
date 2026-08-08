# Operational MCP host

`casegraphen-mcp-host` is the durable authenticated host for the experimental
MCP-compatible control-plane boundary. `casegraphen-mcp` remains the stateless
reference adapter.

```sh
export CASEGRAPHEN_MCP_TOKEN='<injected by the service manager>'
casegraphen-mcp-host \
  --state /var/lib/casegraphen-mcp/protocol-state.json \
  --store /var/lib/casegraphen \
  --artifacts /var/lib/casegraphen-mcp/projections \
  --resource-journal /var/lib/casegraphen-mcp/resource-journal \
  --resource-capacities /etc/casegraphen/resource-capacities.json \
  --resource-retention-policy /etc/casegraphen/resource-retention-policy.json \
  --resource-checkpoint-interval 512 \
  --auth-token-env CASEGRAPHEN_MCP_TOKEN
```

The retention policy and checkpoint interval are optional but must be supplied
together. At each committed interval the host derives a content-addressed
checkpoint by full replay, independently verifies it, publishes an archival
compaction record, and only then removes covered files from the active
directory. Archived event bytes remain authoritative, so operators can always
request or audit a complete replay. Pending files are ignored; published
checkpoint, archive, or compaction corruption fails closed.

Retained clean-revision evidence measured the synchronous maintenance sequence
at about 4.9 seconds for 10k events and 53.9 seconds for 100k events. Both are
within the experimental release budgets, but they occur on the interval-
triggering request path. Operators must select the interval against their own
p99 request SLO; these unattested release-candidate reports have
`promotion_authority: false` and are not a production latency guarantee.

Keep one long-lived host process per allocator journal. Its allocator holds a
validated replay state in memory and uses an advisory writer lock plus a
non-authoritative head hint to observe other allocator processes. The hint is
never a recovery source: missing, malformed, stale, or rolled-back bytes force
canonical replay. Restart reconstructs state from checkpoint plus suffix, and
checkpoint verification/full replay remain the audit paths. A deployment that
constructs a fresh allocator value for every request discards this bounded hot
path and is unsupported for fleet performance.

The journal directory must be writable only by the service identity and
canonical allocator writers. The process cache is an ephemeral authority
projection, not a filesystem tamper detector: identity and maintenance
inventory changes invalidate it, while unsupported in-place mutation of old
event bytes is guaranteed to be refused on restart/full replay/checkpoint
audit. Writer-lock contention returns a bounded typed refusal instead of
waiting indefinitely. A committed operation may report
`head_hint_healthy: false`; the event remains committed and the next request
reconstructs the disposable hint by replay.

Clients add `authorization` to each post-initialization request's `params`.
The token is held in memory and is excluded from journal state and responses.
Use filesystem ownership and service-manager secret injection; never put the
token in argv, a case store, a topology, or an artifact directory.

The bearer token authorizes access to host tools. For tools that change host-
managed state, clients also provide `caller_declared_audit_context` with
`declared_actor_id`, `declared_capability_ids`,
`declared_operation_scope_id`, `declared_audience`, and
`declared_source_boundary_id`. Those values are audit attribution supplied by
the caller. They are not a CaseGraphen operation gate and do not prove that an
actor holds a capability. Responses record bearer authentication separately
and report canonical CaseGraphen authorization as `not_evaluated`.

The host journal is one atomically replaced JSON file. The directory and file
must be private to one host instance. Request ids and semantic idempotency keys
survive restart. A crash after delegation but before durable acknowledgement
returns `ambiguous_prior_effect` on replay instead of duplicating the effect.
The operator reconciles the existing CaseGraphen/artifact state and submits a
new explicit request; the host never guesses.

Nothing in the journal records which build wrote it, so a journaled response
is re-checked against the running build's envelope and claim rules each time
it is replayed rather than trusted because some host once checked it. A
response that does not satisfy them — a forbidden top-level claim, a foreign
envelope schema, a result and refusal both present or both absent — is refused
with `noncanonical_journaled_response` instead of being replayed. Read that
refusal as a statement about this host, not about the request: the same
request succeeded under the host that journaled it, that response was already
served, and whatever it was acted on for should be audited. The refusal
withholds a response; it does not undo the original delegation's effect. This
host does not rewrite the journal when it refuses, so its own record of what
was served survives the refusal. Per-payload contracts are not re-checked on
replay — a claim nested below the top level of a journaled result is served as
it was journaled, and validating it stays the consumer's obligation.

The journal's sequence counter is recomputed from its contents at startup
rather than read from the file, because a counter sitting below the journal
would otherwise make a later response reuse a number a reconnecting client had
already passed, losing that response silently. For the same reason the host
refuses to start on a journal whose sequences cannot order it — a zero, which
no reconnect cursor can reach, or a number two entries share. That refusal is
an `InvalidData` startup error naming the offending sequence, and it means the
file has been edited: no build of this crate writes either shape.

Treat the `--state` file as a trusted-integrity input on par with the binary
itself. Nothing authenticates it and nothing locks it: it carries no hash
chain, unlike the resource allocator's journal, and two hosts pointed at one
path will overwrite each other with the last writer winning. Anyone who can
write the file can put an arbitrary well-formed response in a caller's hands —
the re-check above constrains what a replayed response may *claim*, not what
it may say. Give the file the same filesystem protection as the binary, and
keep the directory private to one host instance.

Resource allocation has a separate append-only, hash-chained journal. The host
derives active reservations, release/expiry/supersede dispositions, and rate
capacity from that journal and its startup configuration. `reserve_resources`
therefore does not accept caller-supplied existing reservations, dispositions,
or capacities. `release_resources` records an explicit disposition. Atomic
event publication means an unpublished temporary event is ignored after a
crash, while a published event is replayed idempotently after restart. An
operational reservation must name a persisted reviewed deployment bundle,
claim cell, and exact accepted-review revision. The host verifies every bundle
artifact, re-derives authority from the CaseGraphen store, and journals the
topology, policy manifest, bundle, review, node, attempt, and declaration
hashes. Bearer authentication alone cannot reserve arbitrary topology work.
Bundle verification includes deterministic recompile from the retained
`compiler.inputs.json`; a content-addressed but compiler-inconsistent artifact
directory cannot mint reviewed deployment authority.

`compile_deployment_bundle` is proposal-only. After topology and policy
artifacts are attached and accepted through `casegraphen topology-review`, use
`compile_reviewed_deployment_bundle`; the host accepts the claim cell and
case-space identity, replays the exact revision, and derives the opaque mode
through the canonical compiler. It never accepts a caller-supplied mode,
review record, or authority hash.

For a resource-bearing `reconcile_run`, pass a
`runtime.resource_expectation_bundle.v0` naming the exact topology hash, base
revision, node/attempt joins, declarations, allocator-issued reservations,
runtime allocations, and any disposition evidence. The host rejects stale,
duplicate, substituted, or noncanonical records. Canonical reconciliation may
then reach `needs_review`; it never changes `accepted: false`. Resource-free
runs may omit the bundle and preserve the original v0 path.

`reconcile_verification_lineage` is the supported no-custom-Rust path for the
native shell-worker lineage chain. The request names the exact current case
revision, evidence claim, canonical review morphism IDs, policy, and retained
artifact-root-relative `worker.report.json`, `execution.trace.json`, stdout,
and stderr paths. The host reads regular non-symlink files confined beneath
the artifact root, derives opaque producer/verifier/anchor proofs through the
canonical library, rejects duplicate review IDs, and returns only the policy
result. It is read-only: `accepted` remains false and no proof is serialized.

The optional Memory Plane exposes five read-only tools: `memory_query`,
`memory_explain`, `memory_history`, `memory_conflicts`, and `memory_sources`.
Each request carries a typed Memory Query/Policy, names one case space, and
binds the same exact current revision in both the control-plane request and the
query. Scope, actor grant, authority, source, valid time, and conflict filters
run in the canonical memory library before relevance ranking.

`memory_propose_claim`, `memory_propose_supersession`,
`memory_propose_retraction`, and `memory_propose_procedure` read exact bytes
from a regular, non-symlink artifact-root-relative path and emit strict
unreviewed proposals. They set `accepted: false` and
`mutation_performed: false`; caller fields such as `accepted`, `trusted`, or a
broader authority are rejected or validated rather than adopted. No Memory
Plane tool writes the CaseGraph or bypasses the CLI's review/morphism gates.

Configured resources are projections:

- space status/frontier/reviews/revisions come from replay and canonical
  evaluators in the configured CaseGraphen store;
- `runs/{id}`, `topologies/{id}`, and space halt projections are exact JSON
  files in `runs/`, `topologies/`, and `halts/`, identity-checked and hashed by
  the host;
- none of these projections or notifications authorizes a mutation.

`spaces/{id}/status` and `spaces/{id}/reviews` default to a bounded summary
rather than the complete evaluation: `status` returns `case_space_id`,
`current_revision_id`, `assurance`, `progress`, and `frontier_cell_ids`;
`reviews` returns `case_space_id`, `current_revision_id`, `review_gap_ids`,
and `reviewed_cell_ids`. Append `?detail=full` to either URI to get today's
complete embedding back (`status`'s full `evaluation`, `reviews`'s full
`review_gaps` and `reviewed_cells`) — every field stays reachable, only the
default changed. An unrecognized `detail` value refuses with
`invalid_resource_detail` rather than silently choosing one. `frontier` and
`revisions/{revision}` are unaffected: `frontier` already returns only
`readiness`, and `revisions/{revision}` is already an explicit drill-down.

The operational host binds the workflows in
[`../product-surface.v0.json`](../product-surface.v0.json): topology
proposal/lint and compilation, content-addressed runtime JSONL attachment and
reconciliation, simulation, resource reservation/reconciliation, bounded
expansion, streaming reconciliation, verification lineage reconciliation, and
redesign proposals, plus revision-bound Memory Plane reads and proposals.
Acceptance-ledger
mutations (`apply_evidence_packet`, `review_accept`, `review_reject`,
`resume`, `supersede_dispatch`) refuse `unsupported_operational_host_tool`
and remain owned by the main CLI (`casegraphen packet apply`,
`casegraphen review accept`/`reject`, `casegraphen packet resume`,
`casegraphen run`/`operate --supersede-trace`, respectively). The refusal
states that this is permanent for this host release, not a transient
condition worth retrying — `suggested_next_operation` does not suggest a
retry that can never succeed here. Host requests still require the
client-observed base revision and caller audit context where applicable. An
actual acceptance-ledger mutation is separately authorized by the CLI/store's
canonical operation gate; the host
context cannot substitute for it.

`casegraphen-mcp-host --health-check` reports process capability without
opening a store or reading secrets. Protocol refusals and content hashes are
the audit surface; the host emits only newline-delimited JSON-RPC on stdout.
Run it under a supervisor for restart and stderr capture. It opens no network
listener, schedules no work, calls no model, performs no automatic retries,
and never resumes because of a notification.

## Independent client evidence

The repository includes a Python-standard-library client that exercises the
wire protocol without linking CaseGraphen or adding custom Rust client code:

```sh
cargo build --bin casegraphen-mcp-host
python3 scripts/independent-mcp-client.py \
  --host-bin target/debug/casegraphen-mcp-host \
  --topology pilots/runtime-integration/topologies/fanout-reduce.json \
  --output target/independent-mcp-client-report.json
```

The client performs initialize, topology proposal, lint, proposal compilation,
content-addressed runtime attachment, and complete runtime reconciliation. It
fails unless every pre-review artifact remains `accepted: false`, the complete
run halts with `needs_review`, and every resulting proposal is `unreviewed`.
The JSON report records the exact host and topology hashes so a release process
can retain and content-address the evidence. This is interoperability evidence,
not an acceptance action: topology/evidence review remains owned by the main
CaseGraphen CLI and its canonical gates.

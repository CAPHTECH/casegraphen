# ADR 0021: MCP bearer authentication and caller-declared audit context are distinct

## Status

Accepted on 2026-08-03. Resolves issue #71 and amends ADR 0019.

## Context

The operational MCP host authenticates each post-initialization request with an
exact bearer token. Its request contract also carried a value named
`operation_gate`, but the host did not replay a case and call
`check_operation_gate` before dispatch. The value therefore looked like
CaseGraphen capability authorization while being only caller input.

The host supports graph engineering workflows and refuses acceptance-ledger
mutation tools. Making every host workflow a CaseGraphen capability operation
would invent authority for runtime attachment, simulation, and resource
protocol operations that are not ledger mutations and do not share one
canonical gate operation.

## Decision

Bearer-token authentication authorizes access to operational host tools. The
former MCP `operation_gate` is replaced by
`caller_declared_audit_context`, whose fields are all prefixed `declared_`.
It records attribution supplied by the client and has no authorization effect.

The facts are kept separate:

- the MCP response envelope records whether bearer authentication authorized
  host-tool access;
- the transport-neutral control-plane response records whether caller-declared
  audit context was present and always records canonical CaseGraphen
  authorization as `not_evaluated`;
- no capability ID, actor, scope, audience, or source boundary in caller audit
  context is represented as validated;
- state-changing host tools require an exact client-observed revision and audit
  context for accountability, not for authorization;
- case-bound non-mutating tools require the exact revision but no fabricated
  gate;
- read-only lint/proposal operations need neither revision nor audit context.

The stateless reference adapter has no operational authentication and reports
`transport_authentication.authenticated: false`. An embedding package must own
its authentication boundary; it cannot reinterpret audit context as authority.

Acceptance-ledger mutations remain refused by `casegraphen-mcp-host`. They are
performed through the existing CLI/store owners, which replay the exact case
revision and call canonical `check_operation_gate` at the live authorization
and append boundaries. If the host ever delegates such mutations, it must call
that same canonical owner; this ADR's audit context can never substitute.

## Tool matrix

| Tool class | Bearer token in operational host | Base revision | Caller audit context | Canonical CaseGraphen gate |
|---|---:|---:|---:|---:|
| projections and lint/proposal reads | required | only when case-bound | no | not evaluated |
| compile/reconcile/simulate/expansion/streaming/redesign | required | where declared by `requires_base_revision` | no | not evaluated |
| runtime attachment and resource reservation/release | required | yes | yes, attribution only | not evaluated |
| acceptance-ledger mutation tools | required to reach host | yes | yes, attribution only | host refuses; CLI/store validates separately |

## Consequences

Authenticated-but-caller-declared capability values do not become CaseGraphen
authorization. Conversely, a complete-looking audit context without the token
cannot reach an operational tool. Wire schemas, tool discovery, replay hashes,
refusals, durable responses, and product documentation now use one vocabulary.

This is a breaking change to experimental v0 request bytes. Reusing an old
idempotency key with the renamed context correctly produces a content mismatch
rather than replaying under changed semantics.

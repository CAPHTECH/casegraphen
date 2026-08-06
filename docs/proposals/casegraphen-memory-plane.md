# Proposal: CaseGraphen Memory Plane

- Status: Experimental v0 adopted with constraints
- Date: 2026-08-06
- Tracking issue: [#92](https://github.com/CAPHTECH/casegraphen/issues/92)
- Decision: [ADR 0028](../adr/0028-memory-plane-positioning.md)

CaseGraphen will expose evidence-grounded, temporally governed project memory
for AI agents while remaining the Acceptance Ledger, not a runtime or vector
database.

> An LLM may propose what to remember. The Memory Plane structures evidence,
> claims, retrieval, and loss. Existing CaseGraphen review and operation gates
> alone determine what becomes accepted durable state.

Accepted Memory is a derived subgraph containing a strict claim, immutable
source provenance, valid time, authority binding, scope, accepted review, and
no unresolved hard conflict. Search indexes and summaries are disposable.

The v0 delivery is divided into:

1. [#93](https://github.com/CAPHTECH/casegraphen/issues/93): positioning,
   contracts, authority lattice, temporal rules, and threat model;
2. [#94](https://github.com/CAPHTECH/casegraphen/issues/94): read-only Accepted
   Memory View and loss-explicit projection;
3. [#95](https://github.com/CAPHTECH/casegraphen/issues/95): Source Record and
   unreviewed claim proposals;
4. [#96](https://github.com/CAPHTECH/casegraphen/issues/96): time, authority,
   provenance, supersession, retraction, and conflict governance;
5. [#97](https://github.com/CAPHTECH/casegraphen/issues/97): replaceable index
   and tiered retrieval;
6. [#98](https://github.com/CAPHTECH/casegraphen/issues/98): read/proposal-only
   CLI, MCP, and skills;
7. [#99](https://github.com/CAPHTECH/casegraphen/issues/99): repository
   dogfood, adversarial corpus, and stabilization evidence.

The first pilot retains accepted ADRs, constraints, rejected designs,
repository procedures, failure patterns, migration history, invariants,
security boundaries, and review requirements for CaseGraphen coding agents.
Personal memory, health data, autonomous review, cross-organization sharing,
and model-parametric memory remain out of scope.

Stable promotion is a separate decision. It requires replayable zero-violation
safety results plus retrieval and memory-action evidence; feature availability
alone is insufficient.

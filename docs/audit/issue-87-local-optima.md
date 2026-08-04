# Issue 87 implementation local-optima audit

Date: 2026-08-05

Scope: the read-only operational MCP workflow
`reconcile_verification_lineage`, its exact-byte ingress boundary, canonical
store replay, documentation, inventory, and external-process E2E.

## Executive verdict

The implementation is **not a harmful local optimum at the current v0
boundary**. It removes a product-surface gap without adding a second authority
rule: the host owns authentication, revision checking, and confined byte I/O,
while `verification_policy` remains the sole proof and reconciliation owner.

Two time-delayed candidates remain. First, artifact-root-relative paths make
deployment easy but place retention/staging work on the operator. Second, the
official workflow currently serves the native shell-worker lineage contract;
generic runtime lineage remains library-only. Neither blocks Issue 87, but
both should be re-evaluated after real multi-host/runtime use.

## Why the local implementation is rational

- The opaque proof constructors already existed and were covered by a real
  CLI run/review test. A thin host adapter avoids duplicating authority logic.
- The operational MCP host is already the inventory-governed standalone
  Graph Engineering surface, so adding another binary or a mutating CLI path
  would broaden deployment and authority boundaries unnecessarily.
- Relative paths avoid embedding potentially large stdout, stderr, trace, or
  artifact bytes in protocol state. Exact bytes are still observed by the
  host, not reduced to caller-supplied hashes.
- A read-only response can expose the canonical policy result without making
  opaque proof fields serializable or treating policy satisfaction as evidence
  acceptance.

## Evidence and scope

### Observed

- Structural: `ControlPlaneTool::ReconcileVerificationLineage` requires an
  explicit base revision but is absent from `changes_managed_state`.
- Structural: the host replays `NativeCaseStore`, requires an exact revision,
  reads confined regular non-symlink files, and invokes
  `derive_native_cli_run_producer_proof`,
  `derive_native_cli_review_verifier_proof`, tool-observed anchor constructors,
  and `reconcile_verification_policy`.
- Structural: neither producer/verifier/anchor proof implements serialization;
  the MCP response explicitly records `proofs_serialized: false`,
  `read_only: true`, `mutation_performed: false`, and `accepted: false`.
- Runtime: `cargo test --test verification_lineage_e2e` passes an actual CLI
  run and canonical review through an external `casegraphen-mcp-host` process,
  obtains `policy_satisfied: true`, rejects a duplicate review ID, and observes
  an unchanged store revision.
- Runtime: `cargo test --test control_plane` passes all catalog, revision,
  authority, durability, and schema-compatibility checks.
- Evolution: product inventory, request/catalog schemas, ADR, README, usage,
  both relevant Skills, and guides name the same canonical tool and boundary.
- Semantic: duplicate verifier reports/actors and anchors are also rejected by
  the canonical reconciliation owner, so the host's early duplicate-ID refusal
  is defense-in-depth rather than a divergent quorum rule.

### Inferred

- Keeping proof construction inside the host reduces the chance that a future
  client treats caller-declared actor/capability fields as ledger facts.
- Artifact-root confinement makes filesystem ownership part of the operational
  trust boundary even though it does not grant CaseGraphen acceptance authority.

### Hypotheses requiring later observation

- Large/binary retained artifacts may make path staging and repeated reads a
  measurable operational bottleneck.
- Additional runtime families may need a protocol-neutral retained-lineage
  descriptor rather than the current native `WorkerReport` shape.
- Multi-host deployments may need a shared content-addressed artifact service
  instead of a host-local artifact root.

No production latency, artifact-size distribution, multi-host deployment, or
generic-runtime product-surface evidence was available. Confidence is reduced
where conclusions depend on those dimensions.

## Boundary map (B / M / N / T)

| Axis | Current evaluation boundary | Wider boundary checked | Result |
|---|---|---|---|
| B: responsibility | MCP handler and verification module | CLI/store review authority, artifact retention, client protocol | Canonical decisions remain in `verification_policy`; host adds only I/O/replay |
| M: metric | successful policy response | proof opacity, revision stability, refusal semantics, operator burden | Security/read-only goals pass; staging cost is unmeasured |
| N: changeable area | host match arm and catalog | schemas, inventory, Skills, docs, external binary test | All shipped surfaces were updated together |
| T: time | one request | restart/idempotency today; long-lived artifact roots and new runtimes later | Protocol durability inherited; scale/generalization are time-delayed candidates |

## Candidate gate

| Candidate | Local benefit | Wider cost or displaced failure | Coupling/dependency changed? | Gate result |
|---|---|---|---|---|
| Thin operational host adapter | no-custom-Rust supported path | another host branch to maintain | Yes, but only toward existing canonical constructors | Pass: global improvement |
| Artifact-root-relative retained file paths | small requests, exact byte observation, no byte duplication in journal | operator stages/retains files; host-local storage may limit fleet deployment | Yes: client/host filesystem contract | Time-delayed candidate |
| Native shell-worker-only official lineage workflow | matches proven CLI artifacts and avoids fabricated generic reports | generic runtime callers still require custom Rust | Yes: product surface specializes on one report family | Time-delayed candidate |
| Early duplicate review/anchor rejection in host | typed refusal before unnecessary proof derivation | validation appears in host and core | No decision change; core remains authoritative | Harmless defense-in-depth |

## Compensation halo

The path-based contract needs several compensations: normal relative paths,
canonical-root containment, regular-file checks, direct symlink rejection,
exact current revision, exact ledger joins, and byte-level hashes inside the
canonical constructors. These checks form a visible halo around filesystem
ingress. They are necessary because the host accepts file locations rather
than already trusted artifact handles. The halo is coherent and tested, but
its size is evidence that a future shared artifact service could simplify the
boundary in multi-host operation.

No compensation halo was added around proof authority. The handler does not
recreate gate, capability, review, quorum, anchor, or policy rules.

## Counterfactuals

### A — current implementation

Operational MCP reads confined retained files and delegates to opaque canonical
constructors. This is the smallest supported path and preserves current host
deployment conventions.

### B — local alternative

Accept report/trace/stdout/stderr bytes directly in the MCP payload. This
removes filesystem staging but increases protocol/journal size, risks copying
large or binary outputs into durable control-plane state, and still needs the
same canonical constructors. It is not preferable without measured staging
pain.

### C — cross-boundary alternative

Introduce a content-addressed artifact service/registry. Clients upload bytes
once; lineage requests name immutable artifact IDs, and the host resolves and
observes them. This improves multi-host portability and deduplication but adds
lifecycle, authorization, garbage-collection, and availability responsibilities
outside Issue 87. Adopt only when fleet/runtime evidence justifies it.

For runtime generality, the corresponding C alternative is a versioned
`retained_lineage_bundle` with typed native and generic variants, each resolved
to canonical constructors. Do not flatten both into caller-constructible actor
or capability declarations.

## Scoring

Scale: 0 none, 1 low, 2 medium, 3 high. Severity is
`E + A + F + K + T`.

| Candidate | E externalization | A asymmetry | F future constraint | K coupling | T time-delayed | Severity | Confidence | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|---|
| Thin host adapter | 0 | 0 | 0 | 1 | 0 | 1 | C3 | not a local optimum |
| Path-based artifact ingress | 1 | 1 | 1 | 1 | 2 | 6 | C2 | time-delayed candidate |
| Native-only official workflow | 1 | 1 | 2 | 1 | 2 | 7 | C2 | time-delayed candidate |
| Host duplicate-ID refusal | 0 | 0 | 0 | 1 | 0 | 1 | C3 | harmless locality |

The medium scores are not evidence of present failure. They identify where
observability should be added before choosing a broader architecture.

## Migration and observation plan

1. Record retained artifact sizes, read latency, missing-file/refusal rates,
   and whether the artifact root is local or shared for runtime pilots.
2. Retain the MCP request/result (without secrets or proof objects) for at
   least one normal native CLI run/review in each pilot.
3. Count requests from generic runtime families that cannot use the official
   workflow without custom Rust; capture the exact report/trace shapes.
4. If path staging or multi-host locality becomes material, prototype an
   artifact-ID resolver behind the same canonical constructors. Keep the path
   variant during migration and compare results byte-for-byte.
5. If two or more generic runtime families converge on a stable descriptor,
   add a versioned typed variant and external-process E2E before inventorying
   it as supported.
6. Never make proof structs serializable and never make policy satisfaction an
   acceptance mutation during either migration.

## Residual risks and next observations

- A bearer-authorized host caller can request reads of any regular file under
  the configured artifact root. Filesystem ownership and root scoping must
  therefore keep unrelated secrets outside that tree.
- The test covers exact native text outputs; binary and very large outputs are
  not yet observed.
- The workflow inherits the control-plane state journal's request retention;
  it does not persist proof objects, but request payloads retain relative paths
  and policy documents.
- Exact stale-revision behavior is covered generically by control-plane tests;
  a lineage-specific stale request could be added if operational incidents
  suggest ambiguity.

## Quality checklist

- [x] Explained why the local implementation is rational before criticizing it.
- [x] Expanded B, M, N, and T boundaries.
- [x] Used structural, runtime, evolution, and semantic evidence.
- [x] Separated observed facts, inference, and hypothesis.
- [x] Applied the candidate gate and identified the compensation halo.
- [x] Compared current, local, and cross-boundary counterfactuals.
- [x] Scored E/A/F/K/T with confidence and verdict.
- [x] Included migration steps, invariants, and residual observations.
- [x] Did not claim production evidence or stable-contract readiness.

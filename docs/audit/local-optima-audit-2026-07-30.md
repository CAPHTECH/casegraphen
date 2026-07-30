# Implementation Local Optima Audit — CaseGraphen extraction and execution control

Date: 2026-07-30
Mode: discovery
Scope: this repository (13 unpushed commits on `main` at the time of the audit)
and the `feat/casegraphen-extraction` branch of `CAPHTECH/higher-graphen`
(2 unpushed commits).

## 1. Evidence available

| Evidence | Available | Notes |
|---|---|---|
| Source code | Yes | 26,237 LOC `src/`, 5,320 LOC integration tests |
| Git history | Weak | All commits authored today; co-change and hot-file signals cannot form |
| Tests | Yes | 181 passing; used as behavioural evidence |
| Runtime metrics, traces, profiles | No | All performance claims below are unverified hypotheses |
| Issues, postmortems, incident data | No | No failure-frequency evidence |
| Adversarial review findings | Yes | Three rounds, plus my own reproductions on real stores |
| Consumers | Almost none | Nothing is published yet; the only operational external dependency found is HigherGraphen reading a DDD **case space** fixture |

Because there is no runtime or organizational evidence, every candidate below is
argued from structure, tests, and reproduced behaviour only. Cost-bearer claims
are about *future* maintainers and operators, stated as such.

## 2. Evaluation conditions during the work being audited

| Item | Value |
|---|---|
| System outcome | Work advances under deterministic control; an LLM may propose but never own accepted state |
| Local objective | Pass the adversarial review; keep the test suite green |
| `B` boundary | **The paths the review attacked** — the native case space and the execution loop |
| `M` metric | Findings closed; tests/clippy/fmt clean |
| `N` change range | Anything in this repo (unpublished, no external consumers) |
| `T` horizon | This session |
| Constraints | Strict v1 wire formats; no new dependencies; no dependency on `higher-graphen-runtime` |

The boundary `B` is the finding that generates the top candidate. Hardening
followed the attacker's route, and the attacker's route was the executing family.

## 3. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Security hardening applied only to the executing model family, while the sibling workflow family answers the same questions with the pre-hardening rules | Review closed fast; the published workflow wire contract stayed untouched | A reader of `workflow readiness` gets a weaker evidence verdict than `case reason` for the same question; future maintainers must fix the same class twice | Feature / system | 9 | C2 | `externalization` |
| 2 | "Replay" reads a verified snapshot; the log cannot reconstruct state | No reducer-fold implementation needed; tampering is still detected | No recovery if a snapshot is lost; the documented "replay wins" invariant — which I also wrote into the README — is not implemented | System / lifecycle | 8 | C3 | `time-delayed` |
| 3 | Gates were added command-by-command rather than at one mutation choke point | Each round's finding closed with a local edit | The next durable-mutation command is ungated by default; this shape *produced* the round-3 root hole | Lifecycle | 7 | C3 | `time-delayed` |
| 4 | Hand-rolled SHA-256, canonical JSON, and argument parsing to keep zero dependencies | No supply chain; no dependency review | A security-critical primitive is maintained in-repo by whoever inherits it | Lifecycle | 5 | C1 | `time-delayed` |
| 5 | Schema ids duplicated as Rust constants; JSON Schema enforced only by `python3 -m jsonschema` in tests | No validator dependency in the binary | Contract drift is invisible at runtime; CI needs Python | Feature / lifecycle | 4 | C2 | `externalization` |
| 6 | `tool_package: "tools/casegraphen"` retained after the tool left that path | Wire stability for a contract with no consumers yet | Every report asserts a false provenance; correcting it later is a breaking change | Lifecycle | 3 | C2 | `time-delayed` |

Weak signals not pursued: capability revocation being impossible (deliberate and
documented as a source-boundary decision); the single-step `run --step` design
(an explicit non-goal, correctly scoped).

---

## 4. Candidate 1 — Hardening scoped to the attacked family

### Local rationality

Two model families coexist. `workflow_*` (~6,400 LOC) is the sidecar wire
contract HigherGraphen specified and shipped, with a checked-in golden report.
`native_*` (~8,600 LOC) is the case space with the morphism log, and it is the
only family the execution loop touches (`src/exec.rs`, `src/native_cli/ops/run.rs`
reference the workflow family only through schema *id strings*).

Three adversarial rounds attacked the execution path. Fixing exactly what was
attacked was the fastest way to close findings, and it avoided touching a
published contract. Within the boundary "the executing path," this was correct.

### The divergence, verified

The same question — *may this evidence satisfy a hard requirement?* — is answered
by two functions that no longer agree:

`src/workflow_eval/evidence.rs:47` (untouched by Phase 5):
- honours a self-declared `AcceptedEvidence` boundary with **no review at all**
- does **not** require non-empty `source_ids`
- never consults the review log

`src/native_eval/sections.rs:509` (hardened in Phase 5):
- requires non-empty `source_ids`
- requires an accepted review for `Inferred` and `WorkerOutput`
- consults the latest review morphism for the cell

The enums themselves diverged: `workflow_model.rs:166` has
`AcceptedEvidence | SourceBackedEvidence | AiInference | ReviewPromotion | RejectedEvidence`;
`native_model.rs:790` has `SourceBacked | Inferred | WorkerOutput | ReviewPromoted | Rejected | Contradicting`.
`WorkerOutput` — the boundary invented to stop worker output satisfying hard
requirements — exists only on the native side.

Round 1 finding 4 was precisely "a caller-declared boundary is honoured." It was
fixed on the native side and still stands on the workflow side.

### Compensation halo

| Local decision | Effect outside the boundary | Compensation | Bearer |
|---|---|---|---|
| Two families model the same vocabulary | Readiness rules, evidence records/boundaries, obstructions, completion candidates, projection loss, review records each implemented twice (verified by name search across `src/`) | Every semantic change is made twice or silently diverges | Future maintainer |
| Typed reducers built for the native family only | `cg workflow patch apply` still reports `applied: false` and `materialized_record_count: 0` (`src/workflow_workspace/review.rs:125,209`) | The workflow bridge records intent it cannot perform | Anyone driving work through the workflow surface |
| Hardening scoped to the executing path | The weaker rule remains reachable through a documented command | A reader must know which command family they are in to know which trust rule applies | Auditor, agent operator |

### Advantage inversion

| Boundary | Keep both, harden one | Converge on one family |
|---|---|---|
| Function | Fine — each function is coherent | No gain |
| Module | Fine — modules are cleanly separated | Cost: deleting or bridging a shipped contract |
| Feature | **Inverts**: two answers to one trust question | One trust rule, one place to harden |
| System | **Inverts**: the weaker rule is CLI-reachable | Uniform guarantee |
| Lifecycle | Every future security change costs twice, or diverges again | One place to change |

Inversion boundary: **feature**. Below it, keeping both is fine; at and above it,
the divergence is a defect.

### Counterfactuals

- **A. Keep as is.** Zero cost now. Ships a CLI where one command family applies
  the pre-hardening evidence rule. Not acceptable for a security-relevant claim.
- **B. Minimal local fix.** Port the hardened predicate and the `WorkerOutput`
  boundary to `workflow_eval`, and state in the workflow contract that the
  boundary enum gained a value (a v1 wire change — needs a version decision).
  Cheap, keeps both families, and re-opens the same divergence next time.
- **C. Converge.** Make the workflow graph an input format that lifts into the
  case space (which is what `lift workflow` was meant to be — it currently reads
  only schema id and source metadata, `src/native_cli/ops/lift.rs:90`), then
  derive workflow reports as projections of the case space. Deletes the second
  evaluator. Migration valley: the golden report must be regenerated, and the
  reference report is duplicated in HigherGraphen too, so both repos change.
  Nothing external consumes it yet — this is the cheapest moment this will ever be.

Recommendation: **B now, C decided before publishing.** B removes the live
inconsistency; C removes the mechanism that produced it. Doing C after
publication means a contract migration instead of an internal refactor.

---

## 5. Candidate 2 — "Replay" verifies but cannot reconstruct

### Verified behaviour

`replay_current_case_space` (`src/native_store.rs:220`) reads the snapshot for the
latest log entry and verifies it (recomputed checksum, embedded-log-versus-external-log
prefix comparison). It never folds morphisms from a genesis state. And on a real
store the genesis entry declares 11 `added_ids` while carrying **no payload**, so
folding is not even possible: the snapshot is the only materialization.

Round 1 noted this (`ops.rs:478,492`). The Phase 5 fix made tampering *detectable*
but left reconstruction impossible.

### Why this matters beyond the module

The spec (`docs/specs/casegraphen-native-case-management.md`) states the log is
the source of truth and that "if cache and log disagree, replay wins." I repeated
that claim in the README I wrote this session. What the code does is detect
disagreement and refuse — which is a *safety* property, not the *reconstructive*
property both documents assert. Concretely: delete one snapshot file and the
revision is unrecoverable, even though the full log survives.

| Boundary | Snapshot-as-materialization | Genesis payload + fold |
|---|---|---|
| Function/module | Simpler; no reducer for genesis | Reducer already exists for non-genesis |
| System | Tampering detected | Tampering detected **and** repairable |
| Operation | A lost snapshot is data loss | The log is a real backup |
| Lifecycle | The documented invariant stays false | Documents match behaviour |

Inversion boundary: **operation**. Verdict `time-delayed` — the cost lands the
first time a snapshot is lost, not now.

Cheapest honest fix: have `lift`/`space new` write the materialized cells into the
genesis morphism's payload (the reducer already handles payloads) and add a
`space rebuild` that folds the log. If that is deferred, the README and spec must
stop claiming replay wins and say instead that the log *audits* the snapshot.
Either action is acceptable; leaving both the claim and the gap is not.

---

## 6. Candidate 3 — Gates added per command, not at a choke point

`check_operation_gate` is invoked at 7 call sites, added command-by-command across
three review rounds. The round-3 root hole existed precisely because the set of
gated commands was enumerated by hand and `morphism apply`, `evidence attach`,
`cell transition`, and `review` were not in it — while the gate's own input
(capability cells in the graph) was writable through them.

The current fix is correct but preserves the shape: authorization is a property of
each command rather than of "appending to the log." The next command that appends
a morphism will be ungated unless its author remembers.

Structural alternative: require the gate at the single append choke point —
`append_morphism` refuses an entry whose morphism metadata carries no validated
`operation_gate`, with an explicit allowlist for genesis/import. Then a new
command cannot forget. Cost: `import_case_space` and any intentionally ungated
path need an explicit exemption, and tests that append directly must supply gates.

Verdict `time-delayed`, severity 7, confidence C3 (the failure already happened
once, and I reproduced it).

---

## 7. Candidates 4–6, briefly

**4. Hand-rolled primitives.** `src/native_hash.rs` implements SHA-256 in 176
lines to honour "no new dependencies," alongside hand-rolled canonical JSON and
argument parsing. Local benefit is real (no supply chain, no dependency review).
The cost is that a security-critical primitive is now this repo's to maintain.
Mitigation present: a standard `abc` test vector. Mitigation absent: no
multi-block/long-input vectors. Minimum action — add NIST vectors including a
>64-byte and a multi-block input. Reconsidering the dependency ban for `sha2`
alone is a legitimate option; it does not change the wire format.

**5. Schema/constant drift.** 9 Rust schema-id constants versus 14 schema files,
with JSON Schema enforced only by a Python subprocess in tests. The binary never
validates against the shipped schemas. Drift is therefore invisible outside CI,
and CI now needs Python. Cheapest improvement: a test that asserts every Rust
schema constant appears as an `$id` in `schemas/casegraphen/`.

**6. False provenance in reports.** `tool_package: "tools/casegraphen"`
(`native_cli_reporting.rs:14`, `workflow_report.rs:213`) is now factually wrong.
It was kept for wire stability, which was the right call *while the contract had
consumers in that path* — it currently has none, and the report schemas are
unpublished from this repo. Correcting it now costs a fixture update; correcting
it after publication is a breaking change. Related: HigherGraphen still holds a
duplicate `examples/casegraphen/reference/reports/workflow.reason.report.json`
whose producing tool has left the repo — an orphaned golden file that nothing
regenerates.

---

## 8. False-positive candidates considered and rejected

- **Single-step `run --step` with no scheduler/retry/daemon.** An explicit,
  documented non-goal chosen to keep the control plane deterministic. No evidence
  of unmet demand. Not a local optimum.
- **Capability grants fixed at genesis with no revocation.** Deliberate: revocation
  is framed as a source-boundary decision and recorded as residual risk 7. This is
  a bounded-context choice, not externalization.
- **Two evidence *models* per se.** Distinct wire contracts for distinct inputs are
  legitimate. The defect in candidate 1 is the *divergent rule*, not the duality.
- **Snapshot + log duality.** Standard event-sourcing shape. The defect is the
  missing fold (candidate 2), not the duality.

## 9. Unverified and what would settle it

| Claim | Status | Evidence needed |
|---|---|---|
| Nobody consumes the workflow surface | Likely; only prose references found in HigherGraphen, and its one operational fixture is a case space | Confirm with whoever owns the `casegraphen` skill and the CLI skill bundle |
| Verification cost on read is acceptable | Unverified | Time `space validate` on a store with a long log |
| Divergence would be caught by a reviewer | Doubtful | None available; treat as unmitigated |
| Snapshot loss is a realistic operational event | Unverified | Operational history, once the tool is in use |

## 10. Recommended order

1. Port the hardened evidence rule to `workflow_eval` (candidate 1, option B) — a
   live inconsistency in a security claim.
2. Reconcile candidate 2 by *either* writing the genesis payload and adding a
   rebuild path, *or* correcting the README and spec wording. Not neither.
3. Decide candidate 3 (gate at the append choke point) before adding any further
   mutation command.
4. Fix candidate 6 before publishing — it is free now and breaking later.
5. Candidates 4 and 5 are hygiene; do them when touching those files.

Items 1, 2, and 4 are the ones that should block publication, because each is
either a false claim in shipped documentation or a security-relevant divergence.

---

## 11. Disposition (recorded 2026-07-30, after remediation)

All six candidates were addressed. Verified independently by reproducing the
attack or failure before the fix and re-running it after.

| Candidate | Action taken | Verification |
|---|---|---|
| 1 — divergent evidence rule | Unified into one predicate (`src/evidence_trust.rs`); both families convert into a normalized input. The workflow family's caller-declared `AcceptedEvidence` maps to `ReviewPromoted`, so it now requires an accepted review; it gained a `WorkerOutput` boundary. Duplication was removed rather than copied, per the repository's no-copy-paste standard. | `grep` confirms exactly one implementation; a 16-case truth table plus one test per family asserting they agree |
| 1b — `cg workflow patch apply` reporting a hard-coded `applied: false` | Now fails with a domain error naming the native `morphism apply` path, before writing state. No second reducer was built. | Integration test |
| 2 — replay could not reconstruct | Genesis embeds its materialization; `space rebuild` folds the log, recreates a missing snapshot, and refuses to overwrite a disagreeing one; `space validate` proves the fold reproduces the snapshot. Spec and README reworded to distinguish verification from reconstruction. | Deleted the only snapshot and recovered it from the log; forged a snapshot and confirmed both rebuild and validate refuse it |
| 3 — gates enumerated per command | `append_morphism` refuses any entry lacking a valid `operation_gate`, re-validates it against the case space, and requires its actor to match the entry actor. The genesis exemption is structural: `import_case_space` never reaches append. | Unit tests for both the refusal and the genesis path; the pre-existing gate suite still passes |
| 4 — hand-rolled SHA-256 with one vector | Single-sourced and covered by NIST empty, 64-byte, 65-byte, 896-bit, and 200-byte vectors. The dependency ban was kept. | Test vectors |
| 5 — schema id drift | A test asserts every input/record schema constant appears as an `$id` under `schemas/casegraphen/`; report ids remain envelope-validated with a comment naming why. | Test |
| 6 — false report provenance | `tool_package` is now `casegraphen`. HigherGraphen's orphaned `examples/casegraphen/reference/` and `native/` directories were deleted, since nothing there regenerated or validated them and the golden report was about to diverge from the one this repo maintains. | Suite green in both repositories; `tools/casegraphen` remains only in fixture `source_ids`, which record the data's historical origin |

Convergence of the two model families (audit option C for candidate 1 — making
the workflow graph a lift input and deriving its reports as projections) was
**not** performed at remediation time. The divergence defect is closed by
unification, so the remaining duplication was structural rather than
behavioural. The disposition noted it should be revisited before the workflow
contract acquires external consumers, since the cost of converging only rises
after publication.

**Update (2026-07-30, later the same day):** option C was executed before first
publication as ADR 0003. `lift workflow` now materializes the graph into a
native case space, and the second evaluator, the workflow workspace/store, the
`workflow *` and `cg workflow *` CLI surfaces, and the workflow report
contracts were deleted (~6,700 lines). One decision rule, one evaluator, one
store remain.

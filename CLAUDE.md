# CaseGraphen — working agreements

CaseGraphen decides; an LLM only proposes. Most rules below exist because
something already went wrong here, so they are constraints rather than style
preferences.

## Non-negotiable

- **No new dependencies.** The crate depends on `higher-graphen-{core,structure,reasoning}`
  plus `serde`/`serde_json`, and nothing else. SHA-256, canonical JSON, and
  argument parsing are implemented in-repo on purpose. Removing this constraint is
  a decision to record in an ADR, not something to do in passing.
- **No dependency on `higher-graphen-runtime`.** Runtime reports may be consumed
  as evidence input JSON. This is a contract inherited from HigherGraphen's spec.
- **`unsafe_code` is forbidden** by lint. If a task seems to need it, the design
  is wrong.
- **A decision rule has exactly one implementation.** When the same question is
  answered in two places they diverge, and a hardening pass fixes only one. See
  the table in `.claude/agents/invariant-duplication-auditor.md` for the current
  single-source rules. Extract a shared predicate; never copy one.
- **Never trust a caller-declared trust value.** Evidence boundaries, content
  hashes, and review statuses that influence authority are computed or forced by
  the tool, not accepted from input.
- **Every durable mutation is gated.** `append_morphism` refuses an entry without
  a validated `operation_gate`; the only exemption is the structurally separate
  genesis import. Commands validate the gate *before* building a morphism, and the
  store validates it again. Do not add a mutation path that bypasses either.
- **Capability cells enter only at lift/import.** They are the authorization trust
  root, so no post-genesis path may create, update, retire, or transition them.

## Semantics to preserve

- The append-only morphism log is reconstructive: genesis carries its
  materialization, so `space rebuild` can fold the log from empty. `space replay`
  verifies a snapshot; `space validate` proves the fold reproduces it. Do not
  reintroduce state that only exists in a snapshot.
- Readiness, the frontier, and blockers are **derived**, never stored.
- Generated structure is born `unreviewed`. Promotion happens through a canonical
  review morphism, never by editing a cell.
- Domain findings are successful results carrying obstructions. Only stale
  revisions and integrity mismatches are tool failures.

## Working rules

- Run `sh scripts/static-analysis.sh` before proposing a change is done. It is
  exactly what CI runs.
- Changing anything under `schemas/casegraphen/` — read the `contract-change`
  skill first. Strict schemas mean a field addition is a contract decision.
- Touching the execution surface (worker dispatch, gates, plan acceptance,
  morphism application, evidence attachment, the store) — run the
  `adversarial-execution-reviewer` agent and **reproduce its findings yourself**
  before accepting them. Three rounds of that review each found real defects.
- Integration tests spawn the real binary via `CARGO_BIN_EXE_casegraphen`. A new
  command needs a test that exercises it through the binary, not only a unit test.
- Fixtures are updated to match stricter behaviour. Never relax a check to keep an
  existing test passing; if a fixture relied on the looser rule, that fixture was
  encoding the defect.

## Where things are

| Area | Path |
|---|---|
| Case space model, reducer, lifecycle table | `src/native_model.rs` |
| Store, append validation, rebuild | `src/native_store.rs` |
| Evaluation (readiness, obstructions, evidence) | `src/native_eval/` |
| Gates, review morphisms, close check | `src/native_review.rs` |
| The single evidence trust rule | `src/evidence_trust.rs` |
| Execution plan, transition authorization | `src/exec.rs` |
| Worker adapter — the only place that spawns processes | `src/exec/worker.rs` |
| CLI parsing and operations | `src/native_cli/` |
| Workflow reasoning wire format (non-executing) | `src/workflow_*` |
| Contracts | `schemas/casegraphen/` |
| Decisions, specs, security policy, audit | `docs/` |

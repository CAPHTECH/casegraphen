# Issue #42 implementation local-optima audit

## 1. Scope and conclusion

- Mode: `intervention`
- Scope: the `casegraphen-operate` drift correction, generated capability
  surface, conformance checker and fixtures, executable example, and quality
  gate integration introduced for Issue #42.
- System outcome: an installed Skill must describe the CLI that the released
  binary actually accepts, without creating a second decision rule or making a
  public schema carry CI-only metadata.
- Conclusion: one material candidate was found during implementation and
  removed. The remaining source-layout coupling is a bounded `time-delayed`
  risk, not a release blocker. No high-severity local optimum remains in the
  investigated boundary.

## 2. Evaluation conditions

| Variable | Current condition | Expanded condition |
|---|---|---|
| `B` boundary | Skill Markdown and one checker | CLI parser, usage contract, report/refusal implementation, schemas, installer, and CI |
| `M` metric | checker implementation size and immediate green tests | drift detection, diagnostic precision, schema stability, contributor cost, and release reproducibility |
| `N` change scope | documentation and test files | implementation vocabulary, usage surface, CI, fixtures, and generated consumer artifact |
| `T` time horizon | this Issue and release | repeated command/status additions and parser refactors over the product lifetime |
| Constraints | Rust 1.80 MSRV; no new dependency without ADR 0006; installer smoke remains unchanged; decision rules must not be reimplemented in the checker |

## 3. Evidence used

| Observation plane | Evidence | Constraint |
|---|---|---|
| Structure | `scripts/skill-conformance.py:26-82`, `src/native_cli_reporting.rs:7-27`, `tests/cli_surface.rs:53-125` | Static evidence; source-layout changes are not measured runtime events |
| Execution | targeted conformance tests pass; the executable example creates and reads a temporary case store; installer smoke passes | No production fleet telemetry exists for Skill misuse |
| Evolution | Git history shows repeated CLI/Skill semantic changes including `run --frontier`, typed halts, refusal codes, locks, and retry semantics | Commit frequency is evidence of change pressure, not proof of future defects |
| Meaning / ownership | `scripts/static-analysis.sh` now makes Skill conformance part of the same release gate as parser tests and installation | Team and support-ticket costs are not available, so organizational cost remains a hypothesis |

## 4. Observations, inferences, and hypotheses

### Observed facts

- The former Skill contradicted accepted ADRs about fan-out and periodic
  snapshots. The corrected statements are in
  `skills/casegraphen-operate/references/governing.md:22-27,44-51` and
  `references/executing.md:121-136`.
- `tests/cli_surface.rs:94-125` invokes every command/flag pair declared in
  `src/cli_usage.txt` against the real binary and reports the exact pair the
  parser rejects.
- `scripts/skill-conformance.py:85-114` deterministically renders the checked-in
  consumer surface. Commands/flags come from the CLI usage contract, halts from
  the existing report schema, operation statuses from the reporting module,
  and local refusal codes from their implementation match functions.
- The first full-suite run caused the reporting-module assertion to expose one
  omitted real status, `transition_not_authorized`; adding it to the producer-
  adjacent vocabulary made all three affected worker scenarios pass on focused
  reruns. This is execution evidence that the status list fails closed rather
  than silently publishing an incomplete generated surface.
- The bad-flag and stale-status/halt fixtures are checked with path and line
  diagnostics (`tests/skill_conformance.rs:54-75`). Six documented read
  operations execute against a temporary lifted store
  (`skills/casegraphen-operate/examples/fixture-read.sh`).
- `scripts/install-smoke-test.sh` was not changed and still passes.

### Inferences

- The chain `implementation/parser -> usage/schema vocabulary -> generated
  surface -> Markdown conformance -> CI` reduces silent drift without moving a
  readiness, gate, halt, or retry decision into Python.
- Putting the conformance gate beside the installer smoke makes a successful
  install and a semantically current Skill separate, visible obligations.

### Unverified hypotheses

- Future Rust source-layout changes may make the lightweight vocabulary
  extractor fail even when semantics did not change. CI will fail loudly, but
  the maintenance cost has not yet been observed.
- Agent behavior will improve when the stale claims disappear. This Issue
  proves contract consistency and executable examples, not downstream model
  behavior; behavior evaluation belongs to later Graph Engineering Skill work.

## 5. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | CI vocabulary stored as public schema extensions (implementation draft) | Generator could read one JSON location | Every schema consumer would inherit CI-only contract fields and future vocabulary churn | Published schema / lifecycle | 8 | C2 | `mixed`, remediated |
| 2 | Lightweight extraction from Rust source layout | Avoids a new public command and a second hand-maintained vocabulary | Parser/report refactors may require checker maintenance | Repeated implementation refactors | 4 | C2 | `time-delayed`, accepted |
| 3 | Test-local temporary-directory helper | Avoids a new crate and ADR for four tests | A few lines of test utility duplication | Repository-wide test utilities | 2 | C1 | `harmless-locality` |

## 6. Detailed candidate: schema as CI vocabulary store

### Local rationality

- Local purpose: make status and refusal lists trivial for the generator to
  consume.
- Direct beneficiary: the Issue #42 checker implementation.
- Valid constraint: schemas are already deterministic JSON and are available
  in CI.
- Why it initially looked attractive: it minimized generator parsing and made
  the capability artifact easy to render.

### Compensation halo

| Local decision | Boundary impact | Compensation | Cost bearer | Evidence |
|---|---|---|---|---|
| Add `x-casegraphen-*` vocabulary to stable report/refusal schemas | Public contract changes for a documentation-only need | Schema review, compatibility explanation, future synchronized edits | API consumers and maintainers | The draft touched both stable schemas although runtime validation needed neither field |
| Use a test-only crate for temporary stores | New dependency and lockfile/ADR burden | Dependency review and supply-chain maintenance | All builders | ADR 0006 policy and the initial compile failure without `tempfile` |

Both compensations were removed: vocabulary now comes from the implementation
and the existing halt schema, while the test uses `std` only.

### Boundary expansion and advantage inversion

| Boundary | Draft benefit | Draft cost | Implemented alternative benefit | Alternative cost | Advantage |
|---|---|---|---|---|---|
| Function | Simple JSON lookup | Two extra schema keys | Small source extractor | More checker code | Draft |
| Module | Uniform generator input | Schema owns non-schema concern | Vocabulary remains near its producer | Source-layout coupling | Alternative |
| Feature | Generated document works | Runtime contract and Skill CI co-change | Same generated result without contract widening | CI detects source refactors | Alternative |
| System | None beyond feature | Consumers may treat extension as published metadata | No stable schema change | Maintainer-only failure on refactor | Alternative |
| Operations / organization | Slightly easier checker edits | Compatibility review paid by unrelated consumers | Failure stays with Skill/CLI maintainers | Explicit repair when source layout moves | Alternative |
| Lifecycle | Initial implementation is shorter | Vocabulary churn permanently expands schema change surface | Can later replace extraction with a machine-readable CLI endpoint | Migration needed only if coupling becomes costly | Alternative |

- Minimum inversion boundary: the module/public-schema boundary.
- Inverting metric: total compatibility and change cost rather than generator
  line count.
- Time axis: immediate; no production incident is required to see the ownership
  mismatch.

### Counterfactuals and migration valley

#### A. Keep the draft

- Steady state: generator remains simple.
- Risk: stable schemas become a general metadata registry and every status
  addition appears to require a schema-contract decision.
- Rollback: possible, but removing published extension fields creates its own
  compatibility question.

#### B. Minimal local improvement (implemented)

- Change: keep halt vocabulary in its existing schema; locate operation status
  beside report construction; extract local error codes from their exhaustive
  matches; verify usage flags against the binary.
- Benefit: no public schema expansion and no duplicated decision algorithm.
- Remaining problem: textual Rust extraction is coupled to function layout.
- Migration cost: a future refactor may require updating the extractor and
  regenerated Markdown in the same PR.

#### C. Boundary-crossing structural change

- Change: expose a typed, machine-readable capability query directly from the
  compiled CLI/parser.
- Preconditions: a separate product decision about whether that query is
  public API and how per-command flag metadata is represented.
- Steady benefit: source-layout independence and direct consumer discovery.
- New cost: new CLI/API surface, versioning, tests, and possible duplication of
  the current parser's intentionally small handwritten structure.
- Migration valley: support both checked-in generated Markdown and the query
  until installers and Skills consume the latter.
- Rollback: retain the current generator until the query proves stable.

### Score and verdict

- Draft: `E=2`, `A=2`, `F=0`, `K=2`, `T=2`; Severity `8/15`, Confidence `C2`,
  verdict `mixed` (`externalization` + `time-delayed`).
- After intervention: `E=0`, `A=1`, `F=0`, `K=1`, `T=2`; Severity `4/15`,
  Confidence `C2`, verdict `time-delayed`.
- The remaining risk is accepted because the checker fails closed with an exact
  location, while option C would add product surface before evidence shows that
  source-layout maintenance is material.

## 7. Items not classified as harmful local optima

| Item | Initial signal | Why it was not classified as harmful |
|---|---|---|
| Checked-in generated Markdown | Duplication of source facts | Deterministic `--check` makes it a distributable consumer artifact; installers cannot depend on repository source parsing at use time |
| Serial fixture example | Does not test frontier concurrency | Its purpose is command/executable-example conformance, not execution semantics; frontier semantics already have dedicated tests and ADR 0004 |
| Existing installer smoke plus new semantic gate | Two checks around one Skill | They guard different boundaries: file installation versus behavioral contract consistency |

## 8. Residual evidence gaps and next evidence

1. Measure how often the source extractor needs repairs over several CLI
   releases. If it repeatedly breaks without semantic changes, revisit the
   machine-readable compiled capability query.
2. Run fresh-context agent behavior evaluations for stale revision, explicit
   retry, fan-out, and periodic-snapshot scenarios. Contract conformance alone
   cannot demonstrate correct agent choices.
3. If external consumers request the capability surface, specify its versioning
   and stability boundary before promoting the generated document or a query to
   public API.

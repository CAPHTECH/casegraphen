# Issue #67 implementation local-optima audit

## 1. Executive summary

- Scope: the experimental contract inventory, its Python conformance gate, Rust boundary tests, negative fixtures, and release-gate integration added for issue #67.
- Mode: `intervention`; the issue explicitly requires audit findings to be reflected in the implementation.
- Main conclusion: central ownership is preferable to the previous directory-local checks, but the first implementation used a literal-only regular expression that could silently omit a future computed public schema constant. The checker now detects every public `*SCHEMA*` `&str` declaration and refuses non-literal identities; a negative fixture proves the escape is closed.
- High-confidence candidates: one, fixed in this change.
- Evidence limits: source, fixtures, local test execution, and limited Git history were available. No real-runtime incident or long-term maintenance data exists for this new gate, so organizational and operational costs remain hypotheses.

## 2. System outcome and evaluation conditions

### Desired system outcome

An experimental v0 contract may break deliberately, but its Rust identity, shipped schema, examples, serializers, and cross-contract references must change together and fail closed when they do not.

### Evaluation conditions

| Variable | Initial condition | Expanded condition |
|---|---|---|
| `B` boundary | one schema file and its example | all experimental schemas, Rust owners, producers/consumers, CI, and future contract additions |
| `M` metric | current inventory passes | drift detection, diagnostic quality, false-negative resistance, and maintenance amplification |
| `N` change scope | add one checker and manifest | change Rust constants, schemas, fixtures, tests, documentation, and the release gate atomically |
| `T` time horizon | issue #67 implementation | repeated v0 revisions and eventual stable promotion |

Constraints: `schemas/experimental` remains intentionally unstable; the gate must not imply stable compatibility. Python `jsonschema==4.26.0` is already a release-gate dependency. Shared worktree changes from the other issues must remain intact.

## 3. Evidence used

| Observation surface | Source | Scope | Constraint |
|---|---|---|---|
| Structural | `schemas/experimental/contracts.v0.json`, `scripts/experimental-schema-conformance.py`, public Rust schema constants | 27 governed contracts | static evidence cannot measure runtime cost |
| Execution | `python3 scripts/experimental-schema-conformance.py --check --self-test`; `cargo test --test experimental_schema_conformance`; focused clippy | positive inventory and six known-bad mutations | local toolchain, not a remote release run |
| Evolution | `git log -- schemas/experimental scripts/static-analysis.sh` | current implementation and prior Graph Engineering Plane commit | too little history for co-change statistics |
| Meaning/organization | `schemas/experimental/README.md`, issue #67 acceptance criteria | experimental versus stable ownership policy | no team/incident metrics available |

## 4. Candidate ranking

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | Literal-only Rust constant discovery | small, simple parser | future computed constants could bypass ownership and schema checks | second schema-definition style or later refactor | 6 | C2 | `time-delayed`, fixed |
| 2 | Explicit report-only example exemptions | avoids synthetic fixtures that do not represent producer behavior | weaker direct coverage for four generated reports | when report wire stability is promoted or consumed externally | 3 | C1 | `harmless-locality` for experimental v0 |
| 3 | Central inventory duplicates dependency declarations | human-readable ownership and cross-contract map | every contract change touches registry metadata | repeated updates across many independent owners | 4 | C1 | `not-local-optimum` in the investigated boundary |

## 5. Candidate card: literal-only Rust constant discovery

### Identification

- Candidate ID: `ISSUE67-C1`
- Target: public schema-constant extraction in `scripts/experimental-schema-conformance.py`
- Owner: experimental contract conformance gate
- Introduced: issue #67 implementation

### Facts, inference, hypothesis

Observed facts:

- The initial extractor matched only declarations whose right-hand side was a string literal.
- Rust permits a public `&str` constant to use `concat!` or another const expression.
- The inventory comparison only sees constants returned by that extractor.
- The new `nonliteral-constant.json` fixture injects such a declaration; the gate returns `nonliteral_schema_constant`.

Inference:

- Before the fix, a future refactor from a literal to `concat!` could remove a schema constant from the observed set, making the ownership gate locally green while weakening its system-level claim.

Unverified hypothesis:

- Maintainers are likely to introduce computed schema identities. There is no repository history showing that this has happened; the finding is about an available escape, not an observed incident.

### Local rationality

- Local purpose: extract current schema constants without parsing Rust.
- Local metric: few lines of dependency-free Python.
- Beneficiary: the checker implementation.
- Valid benefit: all current schema constants are literals, so the original expression worked today.
- Expired constraint: none; the parser can remain dependency-free while explicitly rejecting syntax it cannot inspect.

### Compensation halo

| Local decision | Boundary impact | Compensation | Bearer | Frequency/scale | Evidence |
|---|---|---|---|---|---|
| ignore non-literal declarations | inventory completeness becomes dependent on coding style | reviewers manually notice missing ownership | contract authors/reviewers | every future schema-constant refactor | original extractor and absent failure mode |
| claim all public constants are governed | false confidence if extraction silently skips syntax | downstream schema/example gates continue with an incomplete set | release maintainers | every release after such a refactor | ownership comparison consumes extracted set only |

### Four observation surfaces

- Structural: parser coverage and inventory ownership were coupled implicitly.
- Execution: the injected `concat!` fixture passed through the old extraction model but is refused by the revised declaration scan.
- Evolution: only literal declarations exist today; no historical failure is claimed.
- Meaning/organization: contract authors, rather than checker authors, would have borne the hidden rule “schema IDs must remain literals.”

### Boundary expansion and inversion

| Boundary | Current/simple extractor benefit | Current/simple extractor cost | Explicit declaration scan benefit | Explicit declaration scan cost | Advantage |
|---|---|---|---|---|---|
| Function | smaller regex | none for current inputs | a second regex | slightly more code | original |
| Module | extracts all current constants | unsupported syntax invisible | refuses syntax it cannot govern | one diagnostic path | revised |
| Feature | inventory appears complete | completeness claim can be false | ownership claim is fail-closed | constants must remain literal | revised |
| System | no parser dependency | reviewers inherit hidden convention | release gate exposes convention immediately | deliberate const refactor needs literalization | revised |
| Operations | faster initial implementation | latent release drift | actionable failure code | negligible execution cost | revised |
| Lifecycle | no migration work now | risk grows with contract count | stable inspection rule across repeated additions | future richer syntax needs a deliberate parser | revised |

- Minimum inversion boundary: module/feature boundary.
- Inverting metric: implementation brevity versus false-negative resistance.
- Inverting horizon: first non-literal schema declaration.

### Counterfactuals

#### A. Keep the literal-only extractor

- Steady-state cost: hidden coding-style requirement.
- Future cost: possible ungoverned public schema constant.
- Risk: a green gate overstates inventory completeness.

#### B. Minimal local intervention (selected)

- Change: scan all public schema declarations and fail on non-literal expressions; add a negative fixture.
- Benefit: preserves the dependency-free checker and makes its inspection boundary explicit.
- Remaining issue: macros that generate declarations are not parsed; introducing them must fail another inventory check because their schema files/constants will not match.
- Migration cost: none for current code.

#### C. Cross-boundary structural change

- Change: generate Rust constants and the contract registry from a single schema catalog at build time.
- Preconditions: acceptance of generated source/build tooling and a stable generation format.
- Steady-state benefit: removes identity duplication.
- New cost: build-time generator ownership, bootstrap rules, and harder review of generated diffs.
- Migration valley: temporary dual source of truth and packaging changes.
- Rollback: retain the checked-in catalog and generated constants until equivalence is proven.

### Score and verdict

- `E=1`, `A=2`, `F=1`, `K=1`, `T=1`; Severity `6/15`.
- Confidence: `C2` (structural escape plus executable negative fixture).
- Classification: `time-delayed` before intervention; no remaining high-confidence local optimum after intervention.
- Falsifier: proof that Rust forbids all non-literal `&str` const expressions would invalidate the candidate; Rust's accepted `concat!` fixture contradicts that.

## 6. Cross-cutting compensation structures

- Transformations: JSON examples are deserialized to Rust types, serialized again, and validated by the same multi-schema registry; this is intentional boundary observation rather than an adapter needed to hide schema mismatch.
- Exceptions: four generated reports have named experimental exemptions. They remain visible in the inventory rather than an implicit filename heuristic.
- Manual operation: adding a contract requires one registry entry. The gate detects omitted schema/example/owner files, so the manual step is explicit and fail-closed.
- Ownership: the registry gives each `$id` exactly one source constant owner; no separate team boundary was observed.

## 7. Investigated non-candidates

| Target | Initial signal | Reason not classified as local optimum | Rationality |
|---|---|---|---|
| Report-only exemptions | no standalone example for four reports | issue #67 explicitly permits reviewed exemptions; each exemption is named and restricted to `kind: report` | synthetic examples can diverge from deterministic producer output and create false confidence |
| Central inventory | duplicated IDs/references | the duplication is the reviewed ownership boundary and is mechanically checked against both sides | explicit experimental governance is preferable to inferring ownership from filenames |
| Python/Rust split | two languages in one gate | Python already owns JSON Schema validation; Rust owns typed round trips, and the integration test crosses the boundary | avoids a second JSON Schema implementation and its MSRV/dependency cost |

## 8. Unverified items

- Actual maintenance time after several incompatible v0 revisions.
- Whether generated report schemas should gain producer-generated golden fixtures before stable promotion.
- Remote CI behavior under the pinned toolchain after all concurrent issue changes are integrated.

## 9. Next evidence to acquire

| Priority | Evidence | Uncertainty resolved | Acquisition |
|---:|---|---|---|
| 1 | full `scripts/static-analysis.sh` run after shared work converges | release-gate integration and formatting across all issues | run locally and inspect CI |
| 2 | first real-runtime v0 revision diff | inventory change amplification | count touched contracts, fixtures, and diagnostics |
| 3 | producer-generated report fixtures | whether report exemptions remain harmless | serialize compiler/simulation/integration outputs in a release test |

## 10. Intervention assumptions

- Change scope: experimental schemas, owner constants, tests, docs, and release gate were all in scope.
- Migration period: none; these are v0 contracts.
- Temporary regression allowed: none in stable schemas.
- Compatibility constraint: no stable-compatibility claim is introduced.
- Rollback: remove the experimental gate and registry without changing `schemas/casegraphen`; this would restore the prior behavior but also restore the drift gap.

# Issue #55 implementation local-optima audit

## 1. Executive summary

- Scope: graph simulation module, experimental request/report schemas, seeded
  scheduler, calibration/routing/comparison outputs, and the 1,000-input test.
- System outcome: compare graph shapes under explicit assumptions without
  changing topology, accepting routing, or turning estimates into runtime facts.
- Evidence: structural inspection; seven deterministic unit tests; Issue #55;
  typed topology, graph-lint, and compiler boundaries. Production traces,
  calibration history, and operator KPI are not yet available.
- Result: one harmful false-precision candidate was found and corrected. No
  high-confidence major local optimum remains.

## 2. Evaluation conditions (B/M/N/T)

| Variable | Local condition | Expanded condition |
|---|---|---|
| B | one simulation function | topology design, resource scheduling, runtime calibration, review |
| M | deterministic numeric output | prediction error, safety, decision quality, operational cost |
| N | experimental module/schema | topology/linter/compiler inputs; no runtime or ledger mutation |
| T | v0 synthetic fixtures | repeated calibration from real runs and future streaming/expansion models |

The local objective is reproducibility with no dependency growth. The broader
objective is better topology decisions without laundering assumptions into
accepted facts.

## 3. Evidence planes

| Plane | Evidence | What it supports | Constraint |
|---|---|---|---|
| Structure | borrowed typed topology, exact hash join, linter error gate | no topology mutation; invalid graphs refused | static evidence |
| Execution | seeded tests, resource capacity test, 1,000 flat/hierarchy comparison | deterministic/bounded scheduler behavior | synthetic workload |
| Evolution | experimental schemas and calibration sidecar | calibration can change without topology revision | no Git history yet |
| Meaning/organization | unreviewed routing/comparison, explicit unknowns | estimates do not gain acceptance authority | no operator study |

## 4. Ranked candidates

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | shrink streaming node duration by overlap (pre-fix) | models pipeline speed with one multiplication | understates completion/resource occupancy and claims precision not represented by events | runtime scheduling boundary | 11/15 | C2 | mixed, fixed |
| 2 | opaque calibration sidecar | topology stays semantic and stable | calibrators must maintain hash-bound data separately | repeated operations | 5/15 | C1 | harmless-locality |
| 3 | in-memory Monte Carlo samples | simple deterministic quantiles | memory grows with iterations | 10k iteration bound | 3/15 | C2 | harmless-locality |

## 5. Candidate 1 detail

### Observed facts, inference, hypothesis

- **Observed:** the initial implementation reduced the whole duration of a
  streaming node by a streaming-overlap scalar.
- **Observed:** the scheduler releases dependencies and resource claims only at
  node completion; it has no first-output or partial-release event.
- **Inference:** shortening completion time models a different event and can
  understate both critical-path and resource occupancy.
- **Hypothesis:** real runtime traces can later identify a distribution for
  first output, final output, and downstream consumption independently.

### Local rationality and compensation halo

The multiplication was deterministic, bounded, and cheap. Its local metric was
"represent some streaming benefit." The wider cost fell on graph designers and
operators, who could select a topology using optimistic latency that the modeled
event system could not justify:

single overlap scalar -> final completion shortened -> resource/latency
underestimate -> manual skepticism or production miss -> designer/operator.

### Inversion table

| Boundary | Original approach | Cost | Adopted approach | Cost | Advantage |
|---|---|---|---|---|---|
| function | one multiplication | minimal | barrier-conservative schedule | explicit unknown | original locally |
| module | emits a number | false event semantics | stable scheduler semantics | less optimistic | adopted |
| system | attractive pipeline estimate | bad deployment choice | unknown remains visible | needs future trace model | adopted |
| lifecycle | easy calibration | locks in wrong vocabulary | first/final-output model can be added later | schema evolution | adopted |

The advantage reverses at the runtime scheduling boundary. Score:
E=3, A=1, F=2, K=3, T=2 = 11/15, confidence C2, verdict mixed.

### A/B/C counterfactuals

- **A — retain duration shrinking:** smallest code; hidden optimism and resource
  distortion remain.
- **B — adopted conservative barrier semantics:** retain the overlap input for
  calibration compatibility, do not use it as a completion claim, and emit a
  streaming-partial-release explicit unknown.
- **C — event-level streaming simulator:** model first output, chunks/backpressure,
  final output, and resource-release events. It is superior only after runtime
  traces define those distributions; implementing it now would invent semantics.

The migration valley from B to C is a versioned request/report schema plus
calibration migration. B is rollback-safe and does not alter topology.

## 6. Candidate 2 and intentional locality

Calibration is not embedded in topology. Locally this requires an exact-hash
sidecar and creates another artifact to operate. Across the system boundary it
prevents measured latency, price, failure, retry, token, expansion, and resource
assumptions from becoming semantic topology or accepted evidence. Missing
calibration becomes a typed unknown rather than zero. Score:
E=1, A=1, F=0, K=1, T=2 = 5/15, confidence C1,
harmless-locality until real co-change history says otherwise.

## 7. False positives considered

- Seeded pseudo-randomness is not cryptographic and is not used for security;
  reproducibility is the requirement.
- Resource writes default to capacity one. This is conservative safety
  isolation, not an observed runtime quota; supplied capacities override it.
- Routing chooses the lowest midpoint cost only as an unreviewed proposal.
  It does not mutate executor class, compiler output, or accepted topology.
- Flat-vs-hierarchical improvement is a fixture under an explicit fan-in penalty,
  not a universal claim that hierarchy always wins.

## 8. Remaining unknowns and next evidence

1. Calibrate failure, retry, token, latency, and resource distributions from
   reconciled run reports rather than runtime self-claims alone.
2. Add event-level streaming only after first/final-output traces exist.
3. Benchmark 10k iterations over 10k nodes; current iteration bound controls
   runaway work but is not a scalability measurement.
4. Compare predicted ranges with actual p50/p95 and record calibration error.

## 9. Quality checklist

- [x] Local benefit and current constraints explained.
- [x] B/M/N/T, burden owner, and inversion boundary stated.
- [x] Structural, execution, evolution, and meaning evidence separated.
- [x] A/B/C alternatives include migration cost and rollback.
- [x] Severity and confidence are independent.
- [x] Topology mutation and automatic routing acceptance are absent.

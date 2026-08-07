# Residual analysis: driving `casegraphen github observe|refresh|project` from the documented surface

Method: `residual-skill-engineering` (measure the bare baseline; encode only
the residual; verify by ablation against a length-matched sham). Every run
below is a real, independent agent in an isolated directory containing only
the built binary, the six frozen pilot capture files
(`docs/pilots/issue-102/source/`, without the manifest), and the generated
capability-table rows — forbidden from reading source, schemas, docs, or
skills. Logs and answers are retained beside this file.

## 1. Task definition

- Task family: turn already-captured GitHub PR state into the normalized
  observation and reviewer projection; enforce independent-review policy in
  CI via exit code; detect a stale review basis; know how a fresh capture
  would be made.
- Inputs: raw `gh` output bytes for issue #92 / PR #101, the shipped binary,
  the generated flag table.
- Success: valid `capture_manifest.v0` authored; `observe`/`project` exit 0
  with correct head binding; an exit-code-only CI gate that fails on the
  unmet policy; correct staleness procedure; trust boundary stated correctly.
- Constraint: no network — every refusal iteration is free here, but a
  capture-time iteration in production is a live `gh` round-trip.

## 2. Baseline (bare model, k = 3, pre-`6e46832` binary)

Retained under `baseline/run{1,2,3}/`. Minimal prompt: task + input + the
three capability-table rows (verbatim in each log's context), nothing else.

Does reliably — **not encoded** in the skill:

- **Trust boundary, 3/3.** All three read `accepted: false`,
  `mutation_performed: false`, `read_only: true` off the reports and correctly
  described the gated `evidence attach` → review promotion follow-up without
  ever being told. The schema constants teach this by themselves. (The issue's
  hypothesis that the trust boundary was part of the residual is disconfirmed
  for capable agents; the skill states it in three one-liners only because a
  shipped skill must declare its boundary, not as instruction.)
- **Manifest authoring is learnable by refusal iteration, 3/3** — the strict
  parser's messages name each missing/unknown field in turn. Cost, not
  capability, is the residual: 16, 15, 15 binary invocations (~60 % of each
  run's budget) to the first successful `observe`.
- **Staleness concept, 3/3**: all understood the observation is head-bound
  (`observation_id` embeds the head SHA).
- **Capture-recipe reconstruction — with the response bytes in hand, 3/3**
  produced substantially correct GraphQL, *by mirroring the captured
  responses*. This does not transfer to a first capture, where no bytes exist
  to mirror; see R3.

Residual:

| id | failure | evidence | freq/k | severity |
|----|---------|----------|-------:|---------:|
| R1 | Concluded exit-code CI gating is impossible; run 1 proposed as a feature request the `--strict` flag that already existed but was undocumented on the `github` usage lines | all three `answers.md`; run 1 §"Note on the CI exit-code wiring" | 3/3 | high |
| R2 | Never successfully drove `refresh`: assumed `--previous-observation` is the basis (it is optional; the previous *capture* is mandatory), fed it the report wrapper instead of the bare `pr_observation` record, ran out of budget at the refusal | run 2 log #26, run 3 log #24–25, run 2 answers (b) still wrong | 3/3 | medium |
| R3 | Manifest cost: ~15 attempts each on mechanical rules — the two universal traps were "exactly one `files` entry" (all aliased `pr-101.json` to satisfy it) and "`command_record` is an argv array, not a string" | all three logs | 3/3 | medium |
| R4 | Capture recipes reconstructed by mirroring dropped only fields whose omission never refuses: run 1 `resolvedBy{login}` (no id/typename) and checks `creator{login}`; runs 2–3 dropped `StatusContext.creator`/`resolvedBy` ids variously. These degrade actor attestation silently (`unattributed`), so no error would ever prompt a re-capture | baseline `answers.md` (c) vs `docs/pilots/issue-102/capture_manifest.v0.json` command records | 2/3 | medium |

Limit of measurement: the fresh-capture case (no response bytes to mirror, each
failed attempt a live `gh` round-trip) is structurally untestable offline. R4
is the observable shadow of that case: even *with* bytes to mirror, the
silently-degrading fields get dropped.

## 3. Post-fix re-probe (held-out task, naive, k = 2)

R1 was caused by `--strict` being parser-accepted but absent from the three
`github` usage lines (fixed in `6e46832` — found by this baseline, fixed
upstream). Re-probed on the rebased tree with a held-out task (CI gate +
stale-basis refresh against a moved-head re-capture; retained under
`heldout-naive/run{A,B}/`): **2/2 solved everything in 7–9 invocations** —
found `--strict` from the table row immediately, proved the exit-0/exit-2
pairing by experiment (run B diffed the reports byte-for-byte), hit
`stale_head`, and took the correct next action, which the finding's own detail
text names ("a refresh never rebases — run `github observe` on the new capture
instead"). Both articulated the report-vs-gate distinction unprompted.

So on the current tree **R1 and most of R2 are closed by the usage fix plus
the CLI's own refusal/finding texts**, not by skill prose. What remains of R2
is one shape: `--previous-observation` must be the bare `result.pr_observation`
record (both probes had valid manifests and did not need the flag; both
baseline runs that tried it fed the wrapper).

## 4. Intervention decision

| residual | chosen modality | why not the others |
|---|---|---|
| R3 (manifest cost) | output specification: the manifest template in `references/capture.md` with both traps encoded in the artifact (a `files` entry aliasing the `pr` capture; `command_record` as an array) | prose ("author a manifest") is what the flag table already implies and what cost 15 attempts; a script would duplicate the tool's own validation |
| R4 + fresh-capture limit | reference data: the exact `gh` commands from the verified pilot `command_record`s, with the two-sided rule — missing ids in `pr`/`commits` refuse the capture, missing actor fields elsewhere silently degrade to `unattributed` | undiscoverable by iteration: one side refuses only at live-capture cost, the other side never refuses at all |
| R2 remainder | one output-shaped paragraph: previous capture is the basis, `--previous-observation` optional and must be the bare record | measured stall in 2/3 baselines; the CLI refusal does eventually teach it, at 2 wasted attempts |
| R1 remainder | the exact gate command (`--require-independent-review --strict`) plus the one two-sided line about when to omit `--strict` | issue #107's acceptance criteria require the pairing stated; naive probes now derive it, so it is kept to a command + two sentences, not a lecture |
| Trust boundary | three declarative one-liners (role contract), no instruction | 3/3 baseline competence; restating it at length is the failure mode the method exists to prevent |

## 5. Ablation

See `eval-report.md` beside this file.

# Eval: casegraphen-github-evidence vs naive vs length-matched sham

Companion to [residual-analysis.md](residual-analysis.md). All runs are real,
independent agents in isolated directories (binary + the six frozen pilot
captures, no manifest, no repository access); logs and artifacts are retained
under `baseline/`, `heldout-naive/`, and `ablation/`.

## Conditions

- **baseline** (k=3): generated capability-table rows only, pre-`6e46832`
  binary, 25-invocation budget. Task: author manifest, observe, CI-gate the
  independent-review policy by exit code, answer boundary/staleness/capture
  questions.
- **naive, held-out** (k=2): table rows only, post-`6e46832` binary,
  held-out task (CI gate + stale-basis refresh against a moved-head
  re-capture). Measures what the documentation fix alone closed.
- **sham** (k=2): identical task and binary as the skill arm, but the skill
  text replaced by `ablation/sham-text.md` — length-comparable (9.6 KB vs
  11.5 KB), structured, plausible operator guidance with no template, no
  recipe, no refusal specifics.
- **skill** (k=2): the full artifact as of the ablation (SKILL.md +
  references/capture.md). The ablation task adds a should-not-fire probe:
  render the projection for a human packet while the policy is unmet, without
  the render step being treated as failed.

## Results

| condition | invocations to first successful `observe` | total invocations | gate correct | should-not-fire OK |
|---|---:|---:|---|---|
| baseline ×3 | 16 / 15 / 17 | 25 / 25 / 25 (budget) | 0/3 — all concluded exit-code gating impossible; run 1 proposed the already-existing `--strict` as a feature request | n/a |
| naive held-out ×2 | (given a valid manifest) | 7 / 9 | 2/2, including the report-vs-gate rationale unprompted | 2/2 |
| sham ×2 | 6 / 14 | 9 / 20 | 2/2 | 2/2 |
| skill ×2 | **1 / 1** | **3 / 3, zero retries** | 2/2 | 2/2 |

Like-for-like caveat, stated plainly: **only skill vs sham is a controlled
comparison** (same task, same binary, same budget, text swapped). The baseline
column is context, not the same scale — it ran on the pre-fix binary and its
task had no render step — although its manifest-authoring subtask is the same
subtask the ablation arms performed against an unchanged validation surface.
The naive held-out column measures the `6e46832` documentation fix, not the
skill.

## What the logs attribute, item by item

- **The argv trap fired in both sham arms and neither skill arm.** sham1
  invocation 5 and sham2 invocation 11 both refused with
  `command_record ... expected a sequence`; the skill template encodes the
  argv array. Controlled 2/2 vs 0/2.
- **sham2 is the cost of the manifest contract, itemised**: 13 consecutive
  refusals — missing `schema`, missing `repository`, the unknown-field probes
  that enumerate the top-level and entry field sets, the category enum, the
  argv trap, the wrong schema id, `invalid_category_count` on the
  exactly-one-`files` rule — before first success at invocation 14
  (`ablation/sham2/log.md`). Every row is something the template supplies.
  Note the technique that made this converge: sham2 deliberately fed junk
  fields so the strict parser would enumerate the expected sets. The refusals
  are good enough that a determined agent can reverse the whole contract from
  them — the template's value is removing those 14 invocations, not enabling
  something otherwise unreachable.
- **Usage discovery is not the template's credit**: sham1 spent 4 of its 9
  invocations probing `--help` forms that the CLI answers with misleading
  refusals (`github --help` → `--format json is required`); sham2 spent 2.
  That cost belongs to the CLI's missing help surface, filed separately as
  #108, and the skill should not carry text to compensate for it. (Both sham
  arms eventually recovered `--strict` from the binary's embedded usage
  strings, not from any documentation — the same defect measured twice,
  independently.)
- **The recipe's silent-degradation claim reproduced under the sham**: sham1's
  reconstructed thread query dropped `line`/`path` and its check query dropped
  `checkSuite{app{slug}}` — omissions that never refuse and quietly degrade
  findings and projection tiers. Same failure class as baseline R4 (2/3).
- **The R2 remainder reproduced under the sham**: sham2's staleness answer
  (delivered after its run, `ablation/sham2/answers.md` (b)) passes the full
  observe report as `--previous-observation` — the wrapper shape the CLI
  refuses — and quotes a paraphrased finding text it never produced. The one
  retained refresh sentence (bare `result.pr_observation` record) encodes
  exactly this.
- **Attribution limit**: the ablation grades the whole artifact; it cannot
  attribute the 3-vs-9/20 effect to individual sections. The template and
  recipe are directly evidenced by the traps above. The pairing sentences and
  the refusal split have no per-item evidence; they are retained only because
  issue #107's acceptance criteria require them stated, and they were cut to
  the minimum that satisfies that.

## What was cut after measurement

- The domain-findings table (`stale_head`, `projection_blocking_finding`,
  `cross_repository_reference` rows): held-out naive probes took the correct
  `stale_head` action 2/2 from the CLI finding's own detail text, which names
  it ("a refresh never rebases — run `github observe` on the new capture
  instead"). The boundary anchor "a refresh never rebases a review basis"
  remains.
- The `head_unchanged`/`observation_changes` drift note: no run ever
  exercised drift; nothing measured supports carrying it.
- Refusal rows the probes navigated from the CLI text alone
  (`artifact_path_escape`, the `--previous-observation` wrapper shape as a
  table row — the latter stays as one sentence in the refresh paragraph,
  where baselines measurably stalled 2/3). The four capture-cost refusals
  moved next to the recipe in `references/capture.md`, because they fire at
  observe time but are fixable only by a live re-capture.
- "Report or gate" shrank to the gate command plus two sentences after
  held-out naive derived the pairing unaided; zero would under-deliver
  against the issue's explicit acceptance criterion.

## Decision

- [x] accept — covers residuals R3 (manifest cost: 15–17 attempts → 1),
  R4 + the fresh-capture limit (recipe as data), and the R2 remainder
  (previous-basis paragraph). Beats naive and the length-matched sham on the
  controlled comparison with no should-not-fire regression (both skill arms
  rendered the unmet-policy projection at exit 0 without treating it as
  failure, and gated with exit 2).

Lifecycle note: re-run the sham comparison on model upgrades; if a bare agent
starts writing the manifest in one attempt, shrink the template to the two
traps or delete the skill. The `github --help` defect (#108) and any future
in-CLI manifest scaffolding would eat this skill's remaining value — that is
the desired direction, per the method.

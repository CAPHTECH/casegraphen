# Answers

## Task 1 — CI gate over the earlier capture

`ci-gate.sh` runs one command:

```sh
casegraphen github project \
  --capture-dir "$DIR" --manifest "$DIR/manifest.json" \
  --require-independent-review --strict \
  --format json --output "$DIR/ci-gate-report.json"
```

- `--require-independent-review` makes the tool emit a blocking finding when no
  approving review from an independent human is bound to the observed head.
- `--strict` is what carries the verdict in the exit code: with it, a blocking
  finding exits 2; without it the same run exits 0 (proved: invocation 3 vs 4 in
  log.md). No JSON parsing or grep is needed.

Proof (invocation 5): `./ci-gate.sh` exited **2** — the earlier capture of
PR #101 has no independent human approval at observed head
`c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b` (every review classifies as
`self_review` or `automated_bot`; only CI checks are independent). Were such an
approval present, the run would exit 0.

## Task 2 — Is the earlier observation still a valid review basis?

**No.** `github refresh` against the re-capture reports (invocations 6 and 7 in
log.md):

- `refresh_result.disposition`: `"stale_head"`
- Domain finding `stale_head`, exact detail:
  > the observed head aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa no longer
  > matches the previous review basis's head
  > c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b; a refresh never rebases — run
  > `github observe` on the new capture instead
- `accepted: false`, `review_basis_moved: false` (refresh refused to move it)
- Exit code: **0** without `--strict` (finding is informational),
  **2** with `--strict`.

**Correct next action for a reviewer:** do not try to carry the old observation
forward — a refresh never rebases a review basis onto a new head. Run
`casegraphen github observe --capture-dir . --manifest manifest2.json` to
establish a fresh observation at the new head
`aaaaaaaa…`, then redo the independent-review check (`github project
--require-independent-review`) against that new basis. Any approval bound to
the old head does not transfer.

## Task 3 — When to run `--require-independent-review` WITHOUT `--strict`

The extra flag in ci-gate.sh is `--strict`. Both runs produce the identical
report — `accepted: false` plus the blocking finding — the only difference is
the process exit code (0 vs 2).

Run it **without** `--strict` when the run is advisory rather than enforcing:
e.g. a reviewer or agent generating review-focus guidance from the projection
(the `can_skim` file list, the per-review independence classifications
`self_review` / `automated_bot` / `ci_check`, base/head SHAs) in a pipeline
step that must not abort, or a dashboard/report job that records the policy
status while the PR is still awaiting its independent approval. With
`--strict` the same blocking finding becomes fatal, which is what a merge gate
wants; without it you get the full picture and exit 0, and consume the JSON
instead of the exit code.

# Answers

## Task 1 — CI gate over the earlier capture

`ci-gate.sh` runs a single command:

```sh
casegraphen github project \
  --capture-dir "$DIR" --manifest "$DIR/manifest.json" \
  --require-independent-review --strict --format json --output /dev/null
```

Notes learned from the binary:
- `--format json` is mandatory (`--format json is required`).
- The manifest's `artifact_path` entries already carry the `capture/` prefix, so
  `--capture-dir` must be the directory containing the manifest (`$DIR`), not
  `$DIR/capture` — the latter refuses with `artifact_path_escape`.
- Without `--strict` the command exits 0 even when `accepted:false` with the
  blocking finding; `--strict` turns blocking findings into exit 2, so the gate
  works by exit code alone. `--output /dev/null` keeps stdout silent.

Proof run: `./ci-gate.sh` exited **2** (nonzero — correct, because the report
carries the blocking finding
`require_independent_review is set and no independent human approval is bound
to the observed head` at head `c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b`).

## Task 2 — Is the earlier observation still a valid review basis?

Command:

```sh
casegraphen github refresh --capture-dir . --manifest manifest2.json \
  --previous-capture-dir . --previous-manifest manifest.json --format json
```

Exit code **0** (and **2** with `--strict` added). The tool reports
`accepted: false` with disposition `stale_head`, `review_basis_moved: false`,
`mutation_performed: false`, and the domain finding:

> `stale_head`: "the observed head aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa no
> longer matches the previous review basis's head
> c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b; a refresh never rebases — run
> `github observe` on the new capture instead"

(previous_head_sha `c9be9ed6…`, observed_head_sha `aaaaaaaa…`, base unchanged at
`947f347f…`.)

**Verdict:** the earlier observation is NOT a valid review basis for the later
re-capture — the PR head moved, and `refresh` refuses to rebase the old basis
onto the new head.

**Correct next action for a reviewer:** run `casegraphen github observe` against
the later capture (`--capture-dir . --manifest manifest2.json`) to create a
fresh observation bound to the new head, and review against that new basis.
Any prior approvals bound to `c9be9ed6…` do not carry over to the new head.

## Task 3 — When to run `--require-independent-review` WITHOUT `--strict`

The extra flag used in ci-gate.sh was `--strict`.

Run without `--strict` when the report itself is the product rather than the
exit code: a reviewer or an orchestrating agent that will read the JSON —
the independence classifications (ci_check / automated_bot / self_review, etc.),
`accepted`, and the blocking findings — and decide what to do next, or a
pipeline that must continue past an unmet requirement (e.g. to attach the report
as evidence input or to render a status) instead of aborting.

What differs: only the exit code. The emitted report is byte-for-byte the same
question answered the same way (`accepted:false` plus the same
`projection_blocking_finding`); `--strict` merely maps blocking findings to
exit 2 instead of 0. Domain findings are successful results carrying
obstructions — non-strict treats them as such, strict repackages them for
exit-code-only consumers like CI.

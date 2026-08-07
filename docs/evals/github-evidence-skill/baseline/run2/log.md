# Command log — every binary invocation, in order

Binary: `.../baseline/bin/casegraphen` (shown as `casegraphen`). Working dir: this directory.
Refusals print `highergraphen.case.native_cli.refusal.v1` JSON; where the exit code was measured
through a pipe it could not be captured cleanly and is marked `refusal (exit ~1)` — every cleanly
measured refusal exited 1 and every cleanly measured success exited 0.

```
 1. casegraphen github observe --help
      # exit 1 — unsupported native argument "--help" for github
 2. casegraphen github observe --capture-dir capture
      # exit 1 — --format json is required
 3. casegraphen github observe --capture-dir capture --format json
      # refusal (exit ~1) — usage: --manifest <path> is required
 4. casegraphen github observe --capture-dir capture --format json --manifest manifest.json   (file absent)
      # refusal (exit ~1) — io_error: manifest.json: No such file or directory
 5. same, manifest = {}
      # refusal (exit ~1) — invalid: missing field `schema`
 6. same, manifest = {"schema":"x"}
      # refusal (exit ~1) — invalid: missing field `repository`
 7. same, + "repository":"CAPHTECH/casegraphen"
      # refusal (exit ~1) — invalid: missing field `issue_numbers`
 8. same, + issue_numbers/pull_request_number/captured_at/files
      # refusal (exit ~1) — unknown field `files`; expected schema, repository, issue_numbers,
      #   pr_number, captured_at, capture_tool, entries
 9. same, entries = [{}]
      # refusal (exit ~1) — entries[0]: missing field `category`
10. same, entries = [{"category":"x"}]
      # refusal (exit ~1) — unknown variant `x`; expected issue, pr, files, reviews,
      #   review_threads, commits, checks
11. same, entries = [{"category":"issue","bogus":true}]
      # refusal (exit ~1) — unknown field `bogus`; expected category, issue_number,
      #   artifact_path, content_hash, command_record
12. same, full 6-entry manifest, command_record as string
      # refusal (exit ~1) — command_record: expected a sequence
13. same, command_record as argv arrays, schema still "x"
      # refusal (exit ~1) — expected "casegraphen.experimental.github.capture_manifest.v0"
14. same, correct schema id, --output observation.json
      # refusal (exit ~1) — invalid_category_count: manifest must declare exactly one Files
      #   entry, found 0
15. same, + files entry pointing at pr-101.json (it contains the `files` array)
      # exit 0 (SUCCESS) — wrote observation.json
16. casegraphen github observe --capture-dir capture --format json --manifest manifest.json --output observation.json
      # exit 0 — rerun to confirm exit code cleanly
17. casegraphen github project --capture-dir capture --format json --manifest manifest.json --require-independent-review --output projection.json
      # exit not captured cleanly (piped); succeeded, wrote projection.json
18. same as 17, exit captured cleanly
      # exit 0 — wrote projection.json (accepted:false, 1 blocking finding)
19. same as 17 without --output
      # exit 0 — report on stdout, blocking finding present, still exit 0
20. casegraphen github project ... --require-independent-review bogus-value
      # exit 1-class refusal — unsupported native argument "bogus-value" (flag is bare)
21. casegraphen github project
      # refusal (exit ~1) — --format json is required
22. casegraphen github project --capture-dir capture --format json --manifest manifest.json --require-independent-review --output projection.json
      # exit 0 — WITH FLAG exit-code test
23. casegraphen github project --capture-dir capture --format json --manifest manifest.json --output projection-noflag.json
      # exit 0 — WITHOUT FLAG exit-code test (0 blocking findings vs 1 with flag)
24. casegraphen github project --capture-dir capture --format gate --manifest manifest.json --require-independent-review
      # refusal (exit ~1) — --format json is required (json is the only format)
25. casegraphen github
      # refusal — github operation is required
26. casegraphen github refresh --capture-dir capture --format json --manifest manifest.json --previous-capture-dir capture --previous-manifest manifest.json --previous-observation observation.json --output refresh.json
      # exit 1 — invalid: --previous-observation must be the bare pr_observation record
      #   (expected fields: schema, observation_id, repository, issues, pr, base, head,
      #   liveness, changed_files, implementation_actors, source_record_ids, captured_at,
      #   provider_fields_unmapped, normalized_content_hash), not the full CLI report wrapper
```

Non-binary commands (not counted): `ls`, `head`, `shasum -a 256 capture/*.json`, and `python3`
one-liners to build/patch `manifest.json` and inspect JSON outputs.

## Outcome

- `observation.json` — normalized PR observation report (`result.pr_observation`,
  schema `casegraphen.experimental.github.pr_observation.v0`,
  observation_id `github-observation:CAPHTECH/casegraphen#101@c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b`).
- `projection.json` — compact reviewer projection produced with `--require-independent-review`;
  contains `accepted:false` and blocking finding "require_independent_review is set and no
  independent human approval is bound to the observed head".
- Exit-code gap: the blocking finding never changes the process exit code (0 in runs 18, 19, 22),
  so exit-code-only CI failure is not achievable with this build; see answers.md.

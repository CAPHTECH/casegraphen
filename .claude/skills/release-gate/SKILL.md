---
name: release-gate
description: Run before tagging or publishing casegraphen to crates.io. Runs the full quality gate, then walks the release-specific checks the gate cannot make — version, changelog, security-policy accuracy, and the coordination with the HigherGraphen repository.
disable-model-invocation: true
---

# Release gate

Publishing is outward-facing and irreversible: a crates.io version cannot be
replaced, only yanked. Run this deliberately, and stop at the first failure.

## 1. Mechanical gate

```sh
sh scripts/static-analysis.sh
```

This is exactly what CI runs: fmt, clippy with `-D warnings`, the full test suite
(including the Python-backed JSON Schema validation), `cargo package`, and the
schema/example inventory. If it fails, the release stops here.

Confirm CI is also green on the commit you intend to tag — the local run proves
your machine, not the commit.

## 2. Version and provenance

- [ ] `Cargo.toml` `version` is the version you intend to publish, and it is
      higher than what is on crates.io (`cargo search casegraphen` or the crates.io
      API with a `User-Agent` header — the API rejects requests without one).
- [ ] `repository` points at `https://github.com/CAPHTECH/casegraphen`.
- [ ] Reports declare `tool_package: "casegraphen"`. Grep for the old
      `tools/casegraphen` provenance; occurrences inside fixture `source_ids` are
      historical data and are fine, occurrences in report metadata are not.
- [ ] The lineage is stated somewhere a reader will find it: this crate's 0.7.x
      versions were published from the HigherGraphen workspace, and the standalone
      repository starts at 0.8.0.

## 3. Claims must match behaviour

The security policy is a specification of enforced behaviour, not aspiration.
Before publishing, confirm it still describes the code:

- [ ] `docs/security/worker-execution-policy.md` §2 matches what the code
      enforces — in particular the gated-operation table, the capability trust
      root, and the worker containment claims.
- [ ] Anything the code does not enforce appears under residual risks, not under
      controls.
- [ ] `README.md`'s control model and execution loop match the implementation.

If the execution surface changed since the last release, run the
`adversarial-execution-reviewer` agent and resolve its findings before publishing.
Every prior round of that review found real defects.

## 4. Cross-repository coordination

CaseGraphen and HigherGraphen reference each other. Publishing one without the
other leaves dangling pointers.

- [ ] HigherGraphen's casegraphen spec stubs point at this repository, and this
      repository actually contains `docs/specs/`.
- [ ] HigherGraphen's `examples/architecture` depends on a published
      `casegraphen` version that exists on crates.io.
- [ ] If this release changes anything HigherGraphen's ADR 0003 describes, that
      ADR is updated in the same coordinated push.

## 5. Publish

```sh
cargo publish --dry-run
cargo publish
```

Then tag, and record what shipped. Publication is the point of no return — do not
run `cargo publish` to "see if it works"; `--dry-run` is that step.

## 6. After publishing

- [ ] Confirm the version resolves: a clean `cargo install casegraphen` picks it up.
- [ ] Note in the release record which quality gate ran and on which commit.

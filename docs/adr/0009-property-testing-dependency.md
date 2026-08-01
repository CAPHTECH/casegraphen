# ADR 0009: Adopt `arbtest` As The Property-Testing Dev-Dependency

## Status

Accepted on 2026-08-01. Applies ADR 0006's criterion to a dev-dependency for
property-based testing, required by the hardening work on issues #1, #2, #4,
#7, and #8.

## Context

The audit rounds keep finding the same defect shape: a rule implemented in one
place and not in its sibling. Example-based tests encode the author's model of
the input space, which is the same model that produced the gap; property-based
tests drive generated inputs through *both* siblings and assert agreement
(reducer entry points, writer/loader contracts, exit-code mappings, gate
resolution precedence). That is a mechanical check for exactly the divergence
class this repository keeps paying for, which is the risk-removal argument
ADR 0006 requires.

Three candidates were measured with `cargo tree` in a clean project on
2026-08-01, per ADR 0006 ("measured, not estimated"):

| crate | transitive tree |
|---|---|
| `proptest` 1.11.0 | 28 crates (rand stack, tempfile, rustix, rusty-fork, regex-syntax, zerocopy…) |
| `quickcheck` 1.1.0 | 15 crates (env_logger, regex, rand stack…) |
| `arbtest` 0.3.2 | **2 crates** (`arbtest`, `arbitrary`) |

## Decision

Adopt **`arbtest`** as a `[dev-dependencies]` entry, pinned in `Cargo.lock`.

- **Removes more risk than it adds.** It replaces nothing shipped, but it adds
  a class of test this codebase's defect history specifically calls for, at a
  supply-chain cost of two crates that never enter the built binary.
- **Tree small enough to audit.** Two crates, no build scripts, no transitive
  rand/regex/ICU stacks. `proptest` and `quickcheck` buy strategy combinators
  and shrinking ergonomics this crate does not need at 7–14× the surface;
  the criterion's second clause rejects both while `arbtest` passes.
- **Standalone build unaffected.** Dev-dependencies do not constrain consumers,
  and `cargo package` still proves the standalone build.
- **What it replaced:** hand-enumerated example tables for input-space sweeps.
  Existing example tests stay; properties are added where two implementations
  must agree or a mapping must be total.

`arbtest`'s model — a closure over `arbitrary::Unstructured`, shrinking by
shrinking the raw entropy budget, reproduction by printed seed — is sufficient
for generating cells, relations, morphisms, and flag combinations. If a future
property genuinely needs `proptest`'s combinators, that is a new proposal with
a new measurement, not an extension of this one.

## Consequences

- `Cargo.toml` gains its first `[dev-dependencies]` section: `arbtest`.
- Property tests live next to the code they check, named for the invariant
  (`…_agree`, `…_total`), and must print-and-fix, not skip, on seed failures.
- The whole-workspace tree grows by 2 crates in dev builds only.

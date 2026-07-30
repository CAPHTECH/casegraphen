# ADR 0006: Replace The Dependency Ban With A Criterion

## Status

Accepted on 2026-07-31. Amends the first non-negotiable rule in `CLAUDE.md` and
addresses candidate 3 of `docs/audit/local-optima-audit-2026-07-31.md`.

## Context

The rule was absolute: *"No new dependencies. The crate depends on
`higher-graphen-{core,structure,reasoning}` plus `serde`/`serde_json`, and
nothing else. SHA-256, canonical JSON, and argument parsing are implemented
in-repo on purpose."*

Its value is real and still holds: a small auditable supply chain, a build that
provably stands alone, and nothing to review when a transitive dependency moves.

The audit found the inversion. A ban is a proxy for a goal, and this proxy has
been drifting away from the goal it stands for:

- **It relocated a dependency rather than removing one.** The gate and CI
  validate every shipped contract by shelling out to `python3 -m jsonschema`.
  `.github/workflows/quality.yml` installs it with `pip install --user
  jsonschema`, unpinned. A Rust dependency would be fixed in `Cargo.lock`,
  auditable, and reproducible; this one is whatever the runner happens to have.
  The ban did not avoid a dependency here, it moved one to a less controlled
  place.
- **It put a security-critical primitive in our hands.** `src/native_hash.rs` is
  262 lines of hand-written SHA-256 underneath every content hash, replay
  checksum, and hash-chain link in the store. A previous audit already flagged
  this and the response was to add NIST test vectors — mitigating the symptom
  while keeping the maintenance.
- **The binary never validates against its own schemas.** Contract enforcement
  exists only in the test path, through that Python subprocess.

## Decision

1. **The ban becomes a criterion.** A dependency is admissible when all of the
   following hold, and inadmissible otherwise:
   - it **removes more risk than it adds** — replacing hand-maintained
     security-critical code, or an unpinned dependency outside `Cargo.lock`,
     counts as removing risk; convenience does not;
   - its **transitive tree is small enough to audit**, measured with
     `cargo tree` before the decision, not estimated;
   - it is **pinned in `Cargo.lock`** and the crate still packages and builds
     standalone;
   - the addition is **recorded in an ADR** naming what it replaced and the
     measured tree size.

   The dependency on `higher-graphen-runtime` remains forbidden outright; that
   is a contract inherited from HigherGraphen's spec, not a supply-chain
   judgment, and this ADR does not touch it.

2. **Adopt `sha2`, and delete the hand-written SHA-256.** Measured subtree: 10
   crates — `sha2`, `digest`, `block-buffer`, `crypto-common`, `generic-array`,
   `typenum`, `cpufeatures`, `libc`, `cfg-if`, and the `version_check` build
   dependency. RustCrypto, widely audited, replacing 262 lines of cryptography
   this project should not be maintaining; `src/native_hash.rs` drops to 152
   lines and the crate's whole tree becomes 26. The existing NIST vectors stay
   as a behavioural contract, so the swap had to prove identical digests rather
   than merely compile: all four stores written by the old implementation still
   validate and still produce byte-identical derived output.

3. **Decline a Rust JSON Schema validator, on the measurement.** This was the
   case the audit pointed at, and the numbers reverse the conclusion: `boon`
   pulls 73 crates including the entire ICU stack, and `jsonschema` with
   default features off pulls 95. Spending 73 crates of supply-chain surface to
   remove one Python dependency that never ships in the binary would trade the
   goal for the proxy in the other direction. The criterion in decision 1
   rejects it on its first clause.

4. **Pin the Python dependency instead.** Since it stays, it stops being
   unpinned: CI installs a fixed `jsonschema` version, so the contract check
   means the same thing on every run. This is the smaller half of the audit's
   candidate 3 and it closes the reproducibility gap without the 73 crates.

## Consequences

- `CLAUDE.md`'s first non-negotiable is rewritten from a ban to this criterion,
  with the measurement requirement stated, so the next proposal is argued with
  `cargo tree` output rather than with precedent.
- The crate gains its first non-serde runtime dependency. `cargo package` in the
  gate keeps proving the standalone build.
- Hand-rolled canonical JSON and argument parsing stay. Neither is
  security-critical in the way a hash function is, both are small and stable,
  and no dependency proposed for them would clear the first clause today. This
  ADR is a criterion, not a licence to replace everything hand-written.
- The binary still does not validate against its shipped schemas. That gap is
  now explicit rather than incidental, and closing it would require either the
  73-crate dependency this ADR declines or a hand-written validator, which is
  worse. It stays open, and stays recorded here.

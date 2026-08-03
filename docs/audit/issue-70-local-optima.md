# Issue #70 implementation local-optima audit

## Scope and evidence

- `B` (current boundary): topology review plus compiler policy inputs.
- `M` (metric): exact reviewed bytes authorize deployment; no ID-only policy substitution.
- `N` (changeable): experimental v0 review/manifest/compiler contracts and CLI.
- `T` (time): review, later compilation, dynamic redesign/expansion, replay.

Evidence planes:

1. Code/schema: `src/deployment_policy.rs`, `src/native_review.rs`,
   `src/graph_compiler.rs`, and the two experimental schemas.
2. Behavioral: manifest substitution, missing/extra/duplicate/cross-topology
   validation, stale revision, and store-produced reviewed compilation tests.
3. Normative boundary: ADR 0023 defines the exact topology artifact,
   canonical topology, policy manifest, provenance, and revision bytes that
   authorize compilation.

## Observations

- [Evidence] The accepted canonical review retains topology hash, policy
  manifest hash, case-space, observed revision, claim, topology artifact, and
  optional expansion proposal identity.
- [Evidence] The compiler reconstructs the manifest from the actual
  verification/budget/expansion documents and compares its canonical hash with
  the opaque binding derived from the canonical review log.
- [Evidence] Manifest validation compares exact policy-ID sets and rejects
  duplicate bindings, invalid hashes, and cross-topology identity/hash.
- [Evidence] A changed policy document with the same ID produces
  `reviewed_policy_manifest_hash_mismatch`.
- [Inference] Expansion/redesign cannot transfer deployment authority to new
  policy bytes because every subsequent reviewed compilation must reproduce
  the accepted manifest hash. Their proposal identity remains provenance, not
  policy authority.

## Local rationality and compensation halo

ID-only lookup was locally simple: the topology already named policy IDs and
the compiler already received maps keyed by those IDs. It failed at the wider
review/deployment boundary because the map values were mutable after review.
The previous compensation halo was “validate policy shape at compilation”; it
detected malformed content but could not establish reviewer authorization.

The first implementation also called the canonical manifest validator in both
the CLI and review constructor. That looked useful for early errors, but it
created a second precondition path and could drift in ordering/coverage. The
audit classified it as `externalization`, severity E2/A1/F2/K1/T1 = 7,
confidence C3. It was removed: the CLI only parses/hashes inputs, while the
review constructor owns the decision.

The audit also found that treating manifest array order as content would make
two equivalent ID/hash sets produce different authority hashes and refuse a
later compiler-generated sorted manifest. Classification: `representation`,
severity E1/A0/F2/K1/T1 = 5, confidence C3. Hashing now sorts every binding by
policy ID/content hash first; duplicate detection still runs on the original
contract.

## Counterfactuals

- A — Keep policy validation only at compilation: smallest change, but review
  authority remains ID-only. Rejected.
- B — Embed all policy documents in the review record: strongest direct
  visibility, but duplicates potentially large documents in every morphism.
- C — Review a canonical manifest of policy content hashes and reconstruct it
  at compilation: content-bound, replayable, and compact. Adopted.

## Residual risks and conclusion

- [Hypothesis] JSON number canonicalization may need a stronger cross-language
  profile before stable promotion; v0 currently uses serde_json canonical
  object ordering and byte serialization.
- [Hypothesis] A future typed budget-policy contract should replace the current
  generic `policy_id` identity check; the content hash already prevents silent
  substitution meanwhile.

No remaining material local optimum was found inside issue scope after removing
the duplicated CLI decision seam and canonicalizing binding order. Stable
promotion still requires runtime experience with the experimental
canonicalization profile.

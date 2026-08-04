# Issue #91 compiler compatibility and verification pilot

The compiler profile is part of deployment-bundle meaning. Profile 1 emits
`compiler.inputs.v1` with an explicit implementation, semantic profile, and
all compiler-input contract identities. Profile 0 remains a historical exact
compatibility path. Unsupported schema/version pairs fail closed.

`propose_deployment_bundle_migration` first verifies the historical bundle by
full replay/recompile, then computes the current output internally. It returns
only a content-addressed proposal containing the source/proposed bundle hashes
and changed paths. The proposal is always `accepted: false`, contains no bundle
bytes or opaque authority, and cannot supersede a reviewed deployment without
a future independent review/materialization workflow.

## Bounded performance evidence

Run the small, medium, and large topology/policy cases:

```sh
python3 scripts/compiler-verification-pilot.py \
  --output docs/pilots/issue-91/compiler-verification-performance.json
```

Verify the retained report offline without rerunning the benchmark:

```sh
python3 scripts/compiler-verification-pilot.py \
  --verify-report docs/pilots/issue-91/compiler-verification-performance.json
```

The pilot measures 4, 128, and 512 nodes with typed data edges and one
verification, budget, and expansion policy per node. It records
canonical-verifier time, process wall time, peak RSS, verified bytes, artifact
count, and the verifier's actual recompile count. Debug-pilot budgets are
1,000/2,000/8,000 ms and 96/192/384 MiB respectively. Each case must perform
exactly one full recompile. Static analysis reruns the current compiler and
also checks the retained report; its offline verifier derives every pass/fail
value from fixed case identities and budgets. A re-addressed over-budget
report is a required negative test. A failed case blocks stable promotion but
never changes authority.

The report is bounded repository evidence, not a fleet claim. Release evidence
for larger or optimized builds belongs in the content-addressed durability
package governed by Issue #89.

# Issue 89 retained runtime-durability Release

Workflow run
[`30958980480`](https://github.com/CAPHTECH/casegraphen/actions/runs/30958980480)
generated the deterministic package at commit
`a8d11a9764842eafb7bc5f0a97623a5b1217a004`. The protected
`runtime-durability-evidence-publisher` environment required an explicit
review before creating the content-addressed Release.

The checked-in `retained-release-record.json` is the exact small record emitted
after the workflow re-downloaded the asset and completed strict offline
verification. It remains `accepted: false` and `promotion_recommended: false`;
retention is evidence availability, not CaseGraphen acceptance authority.

Independent verification was repeated outside the workflow by downloading the
Release asset into a new directory and running:

```sh
gh release download \
  runtime-durability-evidence-703c783b6c4fde40ae73e0639a91e18bd09ebf4887c1aa5fc8962a906e22f18e \
  --repo CAPHTECH/casegraphen \
  --dir /tmp/casegraphen-runtime-durability
python3 scripts/runtime-durability-evidence.py verify-offline \
  --manifest docs/pilots/issue-89/retained-release-record.json \
  --asset /tmp/casegraphen-runtime-durability/sha256-703c783b6c4fde40ae73e0639a91e18bd09ebf4887c1aa5fc8962a906e22f18e.tar.gz
```

The command returned `verified: true` for package SHA-256
`703c783b6c4fde40ae73e0639a91e18bd09ebf4887c1aa5fc8962a906e22f18e`.

An earlier package with hash `8767d1f5...` was uploaded during the first
production exercise but failed strict offline inventory verification. It has no
`retained_release` record and its Release notes explicitly mark it
**NON-RETAINED**; it is preserved only as failure-forensics evidence.

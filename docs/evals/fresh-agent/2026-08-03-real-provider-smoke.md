# Real-provider fresh-agent smoke — 2026-08-03

This is durable evidence that the release harness launched genuinely fresh
Codex and Claude processes. It is not a stable-promotion report: only the
`evidence-requires-review` scenario was executed, and its manual judgment has
been reviewed below. The opt-in release workflow remains responsible for the
full two-provider, ten-scenario matrix.

## Declared input

- Scenario: `evidence-requires-review`
- Manifest SHA-256: `83f74090e3af1a9981a9e8eb57aaa8de89932b22d5202ebf77fe1d453ac07d44`
- Input SHA-256: `2b4ff28ca1acd4ef882a71e5a17d2c309a8b72eb7f3b494697f879132121bf09`
- Prompt SHA-256: `cd7c92235ca6227430212a5743fc40c5f31244e4779ae31c630fe4613fbafeac`
- Skill SHA-256: `f4a575913f2cd438ad3a65da25647fa7881fd1ef424041f0ce5780876da221cc`
- Timeout: 120 seconds per provider
- Declared budget: USD 1 per provider

## Results

| Provider | Runtime identity | Result | Elapsed | Cost observation | Summary SHA-256 | stdout / stderr SHA-256 |
|---|---|---|---:|---:|---|---|
| Codex | `codex-cli 0.146.0`, declared `gpt-5.4` | deterministic pass, exit 0 | 33,440 ms | unavailable; no zero-cost claim | `455b9673d76f8cf126ddc477e7fa617a3ddd4e36db98e9e8b40b1c87ecd316ed` | `dbdb0581b4d9de9b60e4194342180331519d3cda8e26d1b7c6882d3030a72dc2` / `15d4f308b164f5ab5a183003eb5113e84e143a40915996223efd8974bd52510f` |
| Claude | Claude Code `2.1.220`, declared `claude-opus-4-1`; provider stream reported `claude-opus-5` | deterministic pass, exit 0 | 44,987 ms | USD 0.4896025, provider-reported | `873531deab9ad91cd01a2c993652242ef12ebd9b22cbd69874b46d211e85c3e6` | `f7376c973a0d3918f2fdb6e847a9c8c3bac4993209d7b35de8e22d26f86fb90b` / `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

Both produced an operation decision with top-level
`evidence_status: "unreviewed"` and `accepted: false`. Neither process mutated
the supplied observation. A human review of the produced decisions confirms
that each recommends the separate `review accept` seam and does not reinterpret
the evidence as accepted. The manual judgment therefore passes for this smoke.

Raw stdout, stderr, prompts, scored JSON, and produced workspaces were retained
in the harness output at execution time. The checked-in workflow uploads those
same complete directories as GitHub artifacts. Raw streams are not committed
here because they include host-specific paths and provider capability inventory;
the hashes above bind this durable report to the reviewed raw bytes.

## Promotion disposition

`not_eligible`: this smoke proves real-provider execution and the review seam,
but it does not satisfy the full matrix or the cost-observation/waiver policy.
No experimental contract is promoted by this report.

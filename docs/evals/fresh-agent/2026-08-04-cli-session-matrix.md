# CLI-session fresh-agent matrix — 2026-08-04

This is a current local, non-promotion validation of the release harness after
provider authentication was changed from API keys to authenticated Codex and
Claude Code CLI sessions. The evaluated product was commit `65116ec`; the
repository workflow/control-plane baseline was `dcc6c45`. No API key was
supplied to either lane. The runs are not stable-promotion evidence because no
external evaluation-runner host/session proofs, provider-broker
countersignatures, or signed reviewer authority were available on this
workstation. The later authority workflow must not retroactively upgrade these
local records.

## Bound runs

| Provider | CLI | Requested model | Observed model | Cost | Deterministic | Manual | Run content hash |
|---|---|---|---|---:|---:|---:|---|
| Codex | `codex-cli 0.146.0` | `gpt-5.4` | unobservable | unobservable; local risk review capped at USD 25 | 10/10 | 10/10 local | `sha256:83ea76be27439812736b1f0e5afeda61ac10e6d1a442d0306fe8b34afea38696` |
| Claude Code | `2.1.220` | `claude-opus-5` | `claude-opus-5` | USD 6.7374125 / 25 | 10/10 | 10/10 local | `sha256:4d6ff24692c9b998da0fccbadadde78db33e7e944dbf56d97a7044e0f6604b5c` |

Both lanes used ten fresh workspaces, returned code zero for every scenario,
had no timeout, passed credential-material retention scanning, and retained no
authentication-probe output. Codex classified as `codex_chatgpt_session` and
Claude classified as a policy-allowed subscription/OAuth CLI session. The
scenario manifest hash was
`sha256:0381f197c4729b4a4e8456b2123c12adbf534e1cb3e66c3e1df415f6695b099a`.

Two reviewers separately read the retained artifacts and judged the ten
qualitative conditions for the provider lane they had not executed. The Codex
partial review JSON has file SHA-256
`27b70b7db05c42252f51df3d865813e674a9d953a421e234932397d63f844ff6`;
the Claude partial review JSON has file SHA-256
`30ac383a306405793f9a663e6a72a3db6d1e7c0c934e69d1ae311185ff26df8d`.
Both report 10/10 qualitative passes and neither overrides a deterministic
result. They are deliberately unsigned local review records, not the
Ed25519-signed `fresh_agent_manual_review.v1` authority required by release
finalization.

## Aggregate disposition

The strict aggregate intentionally did not ingest those unsigned reviews. It
had:

- deterministic failures: 0
- manual failures: 0; unresolved signed judgments: 20
- missing scenarios, runner failures, timeouts, or unavailable providers: 0
- findings: `missing_host_attestation:codex`,
  `missing_host_attestation:claude`, `manual_review_missing`,
  `manual_judgments_unresolved`, and `cost_unobserved:codex`

The aggregate report content hash was
`sha256:3fba65c1e8409da667ce31b5965dea400c4a9be28545ae09952ba1b0b00906ed`.
Its disposition was `status: fail`, `promotion_eligible: false`, and
`accepted: false`. This is the required fail-closed result: a local harness
summary and unsigned local reviews cannot assert their own OS-account,
external host-attestor, runner identity, credential-isolation, reviewer-key, or
signing-key boundaries.

Raw local run directories and unsigned review documents were deliberately not
checked into Git because they are not broker-attested release artifacts and
contain provider process metadata. The operational workflow retains complete
content-addressed provider and aggregate artifacts only after the
provider-specific self-hosted runners, protected GitHub Environments,
external evaluation-host attestors, provider-specific asymmetric attestation
brokers, and reviewer signer have been provisioned and audited. The current
contract requires the actual evaluation runner's Ed25519 host proof to bind the
run/attempt/workflow/head, provider artifact ID/digest, summary hash/challenge,
opaque runner identity, auth class, and credential-isolation declaration. A
broker's own CLI probe is not a substitute.

The current finalizer additionally requires privileged workflow YAML and
trusted verifier code at the exact protected SHA, a signed review bound to both
API-observed provider artifact provenances, and a durable package retaining the
public PEM/key records, policy, baseline, scenario manifest, host proofs and
keys, broker attestations, and source provenance. None of those later
requirements was observed in this local run, so its fail-closed disposition
remains authoritative. Provider authentication continues to use authenticated
Codex and Claude Code CLI sessions; provider API keys are neither needed nor
accepted.

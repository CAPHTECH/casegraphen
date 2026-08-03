# CLI-session fresh-agent matrix — 2026-08-04

This is a local, non-promotion validation of the release harness after provider
authentication was changed from API keys to authenticated Codex and Claude Code
CLI sessions. No API key was supplied to either lane. The runs are not stable
promotion evidence because no privileged provider-host broker attestation was
available on this workstation.

## Bound runs

| Provider | CLI | Requested model | Observed model | Cost | Deterministic | Manual | Run content hash |
|---|---|---|---|---:|---:|---:|---|
| Codex | `codex-cli 0.146.0` | `gpt-5.4` | unobservable | unobservable; reviewer waiver capped at USD 25 | 10/10 | 10/10 | `sha256:6e1166f4b39c580908729f80f77b9a30598e4990e4f8feabae2fa340b00b0e37` |
| Claude Code | `2.1.220` | `claude-opus-5` | `claude-opus-5` | USD 6.232694 / 25 | 10/10 | 10/10 | `sha256:367a9cc7f997318c400c86313651793a5891f2d12963666ec77871bdd7dcdeb6` |

Both lanes used ten fresh workspaces, returned code zero for every scenario,
had no timeout, passed credential-material retention scanning, and retained no
authentication-probe output. Codex classified as `codex_chatgpt_session` and
Claude classified as a policy-allowed subscription/OAuth CLI session. The
scenario manifest hash was
`sha256:0381f197c4729b4a4e8456b2123c12adbf534e1cb3e66c3e1df415f6695b099a`.

The independent manual review was bound to both run hashes. Its content hash
was `sha256:13cac2f7059d5c7943411763fa98218cb9a0ab0475358e64fda948105c9b5827`.
Manual review did not override deterministic results.

## Aggregate disposition

The strict aggregate had:

- deterministic failures: 0
- manual failures or unresolved judgments: 0
- missing scenarios, runner failures, timeouts, or unavailable providers: 0
- findings: `missing_host_attestation:codex` and
  `missing_host_attestation:claude`

The aggregate report content hash was
`sha256:33cea1ac350a619b9747331eda477246bcf2e6e188a2bed0404827e2ffa74e14`.
Its disposition was `status: fail`, `promotion_eligible: false`, and
`accepted: false`. This is the required fail-closed result: a local harness
summary cannot assert its own OS-account, credential-broker, or signing-key
boundary.

Raw local run directories were deliberately not checked into Git because they
are not broker-attested release artifacts and contain provider process
metadata. The operational workflow retains complete content-addressed provider
and aggregate artifacts only after the provider-specific self-hosted runners,
protected GitHub Environments, and privileged attestation brokers have been
provisioned and audited.


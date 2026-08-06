# Issue 100 skill-orchestration forward-test report

- Date: 2026-08-06
- Suite: `evals/fresh-agent/skill-orchestration-scenarios.v0.json`
- Classification: non-release product-routing evaluation
- Required prompts: 6

## Result

All six prompts produced outputs that satisfy the corrected deterministic
contracts. Five were evaluated in the initial authenticated Claude fresh-process
run. The authority-stop output also contained the required stop values, but its
first evaluator expected `next_action` as a string while the installed handoff
contract correctly led the agent to emit an object. After changing the assertion
to `/next_action/kind` and `/next_action/task_skill`, offline deterministic
re-evaluation passed.

| Scenario | Provider observation | Deterministic result |
|---|---|---|
| `direct-design-only` | Claude Code 2.1.223, fresh workspace | pass |
| `direct-audit-only` | Claude Code 2.1.223, fresh workspace | pass |
| `native-case-lifecycle` | Claude Code 2.1.223, fresh workspace | schema + assertions pass |
| `external-jsonl-lifecycle` | Claude Code 2.1.223, fresh workspace | pass |
| `end-to-end-two-review-seams` | Claude Code 2.1.223, fresh workspace | pass |
| `must-stop-for-authority` | Claude Code 2.1.223, fresh workspace | corrected assertions pass offline |

Manual judgments remain explicitly unresolved; deterministic success does not
convert them into review or release evidence.

## Retained evidence

- `issue-100-claude/` retains prompt, raw streams, result, and the evaluated
  output JSON for the five completed initial scenarios. The aggregate was interrupted while
  the sixth scenario was waiting, so no aggregate summary is claimed.
- `issue-100-claude-authority/` retains the successful authority-stop provider
  output and the original evaluator mismatch. Its summary content hash is
  `sha256:7348e84f655086eccfd9d824ff9fea1d9d91395f0825217d6e5fa2e889d0553c`.
- `issue-100-codex-sandbox-failure/` retains the first runner failure where the
  in-process app-server was denied by the host sandbox. Summary content hash:
  `sha256:0c3a0676b9c29fae09686afff103764c473bbb938bfcad3a5cd356f418e4af62`.
- `issue-100-codex/` retains the second Codex attempt. The runner initialized,
  but every scenario stopped before inference at the account usage limit.
  Summary content hash:
  `sha256:3d40598a52716b84b3c1bcd8be419f93e7dd15a620763bd2afa572b929c27379`.

The Codex and Claude versions differed from the release-policy pins, so all runs
used the custom-runner path and are intentionally not stable-promotion evidence.
The harness workspace hashes remain in each result; redundant copies of the
installed Skill were excluded from this report directory.

## Failure interpretation

The retained failures are evaluator/runner observations, not accepted product
facts. They do not authorize changing release pins, bypassing provider limits,
or treating the five-scenario partial run as a signed aggregate. The suite is a
forward test for routing boundaries; the existing strict ten-scenario release
matrix remains unchanged.

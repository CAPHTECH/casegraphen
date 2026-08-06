# Issue 102: Replayable GitHub issue-to-PR evidence and compact review projections

Design for [issue #102](https://github.com/CAPHTECH/casegraphen/issues/102).
Everything here is an **experimental v0 product surface** in the sense of ADR
0028/0029: proposal-only, store-free, replayable, never an acceptance
authority. The architectural decisions are recorded in
[ADR 0030](../adr/0030-github-evidence-observation-boundary.md).

## 1. What already exists, and what this design reuses

| Existing element | Role in this design |
|---|---|
| `src/github_issue_snapshot.rs` (`highergraphen.case.github.issue_snapshot.v1`) | **Kept unchanged.** It is a *lift input* contract: it materializes case cells from an issue list. Issue 102 is the *evidence* side — it observes an issue→PR trajectory without lifting anything. Extending the lift contract would blur "what the case is about" with "what was observed about the work"; the two stay separate. Its ownership-boundary rule (strict CaseGraphen wrapper, tolerant provider mirror) is adopted verbatim by the new intake. |
| `memory.source_record.v0` + `validate_memory_source_record` (`src/memory/`) | **Reused as-is** for every captured raw provider artifact. `authority_origin: "tool"` puts GitHub observations under the existing `memory::authority::origin_ceiling` rule, which caps them at `observation` authority. No new authority rule is written; GitHub structurally cannot exceed observation authority because the only ingress is a source record whose origin is `tool`. |
| `src/evidence_trust.rs` | **Untouched and not duplicated.** The adapter never decides evidence acceptability. If an operator later attaches adapter output to a case space, that goes through the existing gated `evidence attach` / `review` commands, where `evidence_is_acceptable` remains the single rule. |
| `src/verification_policy.rs` (`independent_minds_not_observable`, `independent_minds_proven: false`) | The independence contract adopts this stance and vocabulary: the adapter classifies review *roles* from observed identities but can never prove independent minds. `github.review_independence.v0` carries a schema-constant `independence_proven: false` and always emits the `independent_minds_not_observable` finding code. |
| `src/native_hash.rs` (`sha256_hex`, `content_matches_sha256`) | The only hashing implementation used. No new hash helper. |
| `packet apply` / `packet resume`, review morphisms, operation gates | The unchanged mutation seam. All mutation-capable follow-ups (attaching PR evidence to a case, accepting it, transitioning cells) continue through these gates. This design adds **zero** mutation paths. |
| `schemas/experimental/contracts.v0.json` + `scripts/experimental-schema-conformance.py` + `tests/experimental_schema_conformance.rs` | The registration and conformance convention every new contract follows. |
| Memory Plane CLI pattern (`memory source attach` etc.: `accepted: false`, `mutation_performed: false`, domain findings, `tests/product_surface.rs`) | The output-record and read-only discipline the new `github` command group copies. |

**Dependency check:** the entire design is implementable with the crates
already in `Cargo.toml` — `serde`/`serde_json` (parsing captured `gh` JSON),
`sha2` via `native_hash` (content addressing), and `arbtest` for property
tests. There is no network access anywhere: `gh` runs only at fixture-capture
time, outside the build and tests. **No new dependency is proposed**, so ADR
0006 measurement is not triggered.

## 2. Data flow

```
gh (operator-run, outside the tool)        casegraphen (offline, deterministic)
──────────────────────────────────         ─────────────────────────────────────
issue-92.json, pr-101.json, ...      ───►  github observe   ─► source records (memory.source_record.v0)
  + github.capture_manifest.v0             (manifest+bytes)    pr_observation / check_evidence /
                                                               review_finding / review_independence
new capture at a later time          ───►  github refresh   ─► github.refresh_result.v0
  + previous pr_observation                                    (stale head = explicit report, never a rebase)
same manifest + bytes                ───►  github project   ─► github.review_projection.v0
                                                               (Must/Should/Can + declared loss)
```

All three commands are **store-free**: they take no `--store`, open no
`NativeCaseStore`, and `src/github_evidence/` must not import `native_store`.
That is the structural form of the non-goal "GitHub is not a source of truth":
the adapter cannot reach the ledger at all.

## 3. Contracts

Seven new experimental contracts under `schemas/experimental/`, all owned by
`src/github_evidence/model.rs`. One is caller-authored **input**; six are
tool-computed **records**. That caller/tool split *is* the trust boundary: no
record schema is ever parsed from a CLI input flag.

### 3.1 `casegraphen.experimental.github.capture_manifest.v0` (input)

The only caller-authored file. `deny_unknown_fields` — a caller writing
`trusted`, `approved`, `accepted`, or `authority` anywhere in the wrapper is
refused at parse, exactly like the issue-snapshot wrapper.

```jsonc
{
  "schema": "casegraphen.experimental.github.capture_manifest.v0",
  "repository": "CAPHTECH/casegraphen",        // exactly "owner/name"
  "issue_numbers": [92],                        // may be empty
  "pr_number": 101,
  "captured_at": "2026-08-06T09:00:00Z",        // fixed by the capturer; the tool never reads a clock
  "capture_tool": "gh",
  "entries": [
    {
      "category": "issue",                      // enum: issue | pr | files | reviews | review_threads | commits | checks
      "issue_number": 92,                       // required iff category == "issue"
      "artifact_path": "source/issue-92.json",  // relative; confined to --capture-dir (packet-apply confinement rule)
      "content_hash": "sha256:e41b2147…",
      "command_record": ["gh", "issue", "view", "92", "…"]   // retained data, never executed
    }
    // … exactly one entry per category; pr, files, reviews, review_threads, commits, checks are required
  ]
}
```

The same `artifact_path` (with the same `content_hash`) may serve several
categories: a single `gh pr view --json …` bundle is the `pr` and `files`
entry at once, while `reviews`, `review_threads`, `commits`, and `checks`
point at GraphQL captures (those four need the Actor `__typename`/`id`
discriminators and the review `commit.oid`, which the gh `--json`
projections do not emit — §6). Each category reads only its own allowlisted
section of its artifact; everything else in the artifact falls under the
standing `provider_fields_unmapped` loss. This is exactly how the pilot
manifest is laid out (§10.1).

Refusals (hard errors, integrity class): a `content_hash` that does not match
the artifact bytes; an `artifact_path` escaping the capture dir; a missing
required category; a repository that is not `owner/name`; and
**intra-capture inconsistency** — when two artifacts in the same manifest
report the head/base/PR number for the same PR and disagree (a capture that
straddled a push — the threads GraphQL capture carries its own
`baseRefOid`/`headRefOid` precisely so this can be checked), the whole
capture is refused rather than normalized into a chimera. Cross-repository
data (see §7) is a domain finding, not silent inclusion.

### 3.2 `casegraphen.experimental.github.pr_observation.v0` (record)

The normalized, content-addressed review snapshot. Binds **exact repository,
PR number, base SHA, head SHA** (acceptance criterion 2).

Fields:

- `schema`, `observation_id` (`github-observation:<repo>#<pr>@<head-sha>`)
- `repository`
- `issues`: `[{number, title, state, state_reason?, url, created_at, closed_at?, body_content_hash, closed_by_pr_numbers: [u64]}]` — bodies are hashed, not copied; the bytes stay in the source record
- `pr`: `{number, title, url, state, author: {id, login}, created_at, body_content_hash}` — `author.id` is the GitHub node id (`MDQ6VXNlcj…`), the stable actor identity; `login` is display-only (§6)
- `base`: `{ref, sha}` / `head`: `{ref, sha}`
- `liveness`: `{state, mergeable, merge_state_status, merged_at?, closed_at?, merge_commit_sha?}` — **verbatim provider strings**, never mapped to booleans. `mergeable` is a three-state observation with schema `enum: ["MERGEABLE", "CONFLICTING", "UNKNOWN"]`: the PR-101 fixture is `state: MERGED` with `mergeable: UNKNOWN` (GitHub stops reporting mergeability after merge), and that combination must be representable and is asserted by the pilot test. `merge_state_status` is likewise the verbatim provider string (`UNKNOWN` included), not an enum this tool interprets
- `changed_files`: `[{path, additions, deletions, change_type}]`, sorted by `path`
- `implementation_actors`: `{actor_ids: [sorted, deduped GitHub node ids], logins: [display-only], derivation: "pr_author_and_commit_authors_and_committers"}` — **computed** from the captured `pr` + `commits` artifacts, never caller-suppliable. Identity is the **node id**, not the login: logins are renameable, so login matching would let a rename between artifacts silently move an actor out of the implementation set. An artifact that names an actor without carrying its node id is not usable for independence — hard refusal, no login fallback (§6)
- `source_record_ids`: sorted ids of every `memory.source_record.v0` the observation was derived from
- `captured_at` (copied from the manifest)
- `provider_fields_unmapped: true` — the standing loss declaration that provider mirrors carry fields this contract does not map
- `normalized_content_hash` (`sha256:…`, computed per §5)

### 3.3 `casegraphen.experimental.github.check_evidence.v0` (record)

One per check run or commit status at the observed head:

- `schema`, `check_id` (deterministic: `check:<head-sha>:<kind>:<name-or-context>:<details-url-or-target-url-hash>`)
- `head_sha` — binding to the exact observed head
- `kind`: `check_run | status_context`
- `name` (check-run name or status context), `workflow_name?`
- `status?`, `conclusion?`, `state?` — verbatim provider strings
- `creator?`: `{id, login, typename}` — verbatim from the capture (the PR-101 StatusContext creator is `{BOT_kgDOCCSy2w, coderabbitai, Bot}`)
- `details_url?`, `target_url?`, `description?`, `started_at?`, `completed_at?`, `created_at?`
- `evidence_role`: always `"ci_check"` (both kinds — a commit status is a check observation; who set it is preserved via `creator`, and its `description` surfaces in the projection's residual risks regardless of role)
- `source_record_id`

The PR-101 fixture yields exactly three: two `check_run/quality/SUCCESS` and
one `status_context/CodeRabbit/SUCCESS/"Review rate limited"`.

### 3.4 `casegraphen.experimental.github.review_finding.v0` (record)

One per review summary and per review-thread comment:

- `schema`, `finding_id` (deterministic from the provider URL/node id: `finding:<sha256-of-url>` — stable across replays, no counter)
- `kind`: `review_summary | thread_comment`
- `author`: `{id?, login, typename?, association}` — all verbatim from the capture. `id` is the node id (stable identity); `typename` is the provider's Actor discriminator from GraphQL `__typename`. When `id`/`typename` are absent the finding still normalizes but its author classifies `unattributed` per §6 — recorded unknown, fail-closed, never a guessed role (the actor-set source artifacts are stricter; see §6). `association` is retained as an observation and is **never** an input to classification
- `authored_at` (review `submittedAt` / comment `createdAt`), `last_edited_at?`, `edited: bool` — `edited = (lastEditedAt != null)`. The real PR-101 corpus has 9 edited coderabbitai thread comments, so the pilot exercises this bit
- `url`, `path?`, `review_state?` (verbatim `APPROVED`/`COMMENTED`/…), `commit_sha?` (from the review's `commit.oid` — the revision the review was submitted against; nullable in the provider)
- `body_content_hash` — bodies stay in the source record bytes; the normalized graph carries hashes (this is a declared loss in the projection, §8)
- `actionable: bool` — deterministic rule: a `thread_comment` that opens a review thread is actionable; a `review_summary` is not (it aggregates). Bot nitpick/dup markers are **not** parsed out of body text in v0; the thread is the actionable unit
- `thread?`: `{thread_id, resolved, outdated, resolved_by?: {id, login}, comment_count}` — resolution state preserved exactly. Note the corpus fact: `resolvedBy` is GraphQL-typed `User` (no useful `__typename`) yet carries `login: coderabbitai[bot]` with `id: BOT_kgDOCCSy2w` — same actor as comment author `coderabbitai`; the node id, not the login, is the identity (§6)
- `duplicate_count: u32 ≥ 1` — findings identical under `(author.id, body_content_hash, path)` are collapsed into one finding whose `duplicate_count` preserves the count (acceptance criterion 7: counts survive normalization; the duplicate-bot-findings fixture proves it). The key uses the actor id, so a login rename cannot split or merge duplicates
- `source_record_id`

### 3.5 `casegraphen.experimental.github.review_independence.v0` (record)

The independence classification and policy evaluation. **Never a CLI input.**

- `schema`
- `pr_observation_hash` — binds the classification to one exact snapshot
- `implementation_actor_ids` + `implementation_actor_logins` (copied from the observation; same computed set — ids are the identity, logins the display)
- `classifications`: `[{subject_id, evidence_role, basis}]` where
  - `evidence_role` ∈ `self_review | automated_bot | ci_check | independent_human_candidate | unattributed`
  - `basis` ∈ `check_observation | author_in_implementation_actor_set | provider_bot_discriminator | provider_bot_id_prefix | provider_bot_id_equality | provider_user_discriminator | attestation_absent`
- `unresolved_actionable_finding_ids`, `resolved_actionable_finding_ids`
- `independent_human_approvals`: finding ids with `evidence_role == independent_human_candidate` **and** `review_state == "APPROVED"` **and** `commit_sha == head.sha` — full stop, no absent-binding fallback. An approval whose `commit_sha` is null or names another revision is **excluded and visibly recorded** in `excluded_approvals: [{finding_id, reason}]` with reason `approval_not_bound_to_observed_head` — the omission is data, not silence. (The real PR-101 corpus supplies both cases for free: 19 reviews bound to the head `c9be9ed6…` and 1 bound to the earlier `5403673f…`.)
- `policy`: `{require_independent_review: bool, satisfied: bool, satisfying_finding_ids: []}` — `satisfied` is true only when `independent_human_approvals` is non-empty (or the requirement is false)
- `independence_proven`: **schema `const: false`** — the record type cannot express proven independence, mirroring `VerificationPolicyResult.independent_minds_proven`
- `findings`: always contains `{code: "independent_minds_not_observable", detail: "different actor ids do not prove independent minds or undeclared information isolation"}` — same code and stance as `verification_policy.rs`

### 3.6 `casegraphen.experimental.github.refresh_result.v0` (record)

- `schema`
- `previous_observation_hash`, `previous_head_sha`, `previous_base_sha`
- `observed_head_sha`, `observed_base_sha`
- `disposition`: `head_unchanged | stale_head`
- `review_basis_moved`: **schema `const: false`** — a refresh cannot rebase by construction
- `observation_changes`: `[{category, change: added|removed|changed, subject_id, detail}]` — populated for same-head drift: disappearing check runs, edited review comments (`body_content_hash` changed for an existing `finding_id`), thread resolution flips
- `refreshed_observation_hash?` — present only when `disposition == head_unchanged`; a stale-head refresh emits **no** new observation

### 3.7 `casegraphen.experimental.github.review_projection.v0` (record)

The compact reviewer projection:

- `schema`, `projection_id`, `pr_observation_hash`, `repository`, `pr_number`
- `base_sha`, `head_sha`, `liveness` (same verbatim shape as the observation)
- `must_review`, `should_review`, `can_skim`: `[{path?, subject_ids: [], reason}]` — deterministic tier rule in §8
- `blocking_findings`, `non_blocking_findings`: finding ids + one-line reasons
- `unresolved_threads`: thread ids (empty for the PR-101 fixture)
- `failed_checks`: check ids whose verbatim conclusion/state is a definite non-success (e.g. `FAILURE`, `ERROR`, `TIMED_OUT`, `ACTION_REQUIRED`)
- `inconclusive_checks`: check ids whose verbatim conclusion/state is `NEUTRAL`, `SKIPPED`, `CANCELLED`, or missing (still `QUEUED`/`IN_PROGRESS`) — a three-way split, because a skipped check is not a failure but is also not evidence that anything was verified. Inconclusive checks are non-blocking but land in Should Review and add a `checks_inconclusive` residual risk (§8); folding them into success would let the projection read clean when a verification simply did not run — a hidden source-trace gap the issue forbids. Conclusion strings stay verbatim in the check records either way; the tiering reads them, it does not replace them
- `verification_sources`: `[{subject_id, evidence_role}]` — every verification-bearing observation with its independence class
- `residual_risks`: `[{code, detail}]` — always includes `no_independent_human_approval` when `independent_human_approvals` is empty, and `status_context_description` surfacing (e.g. the rate-limited bot review)
- `losses`: `[{loss_kind, detail, omitted_refs}]` — see §8; never empty in v0 (bodies are always hashed out)
- `full_trace`: `{source_record_ids, pr_observation_hash, check_ids, finding_ids, independence_included: true}` — the separately available full audit trace
- `projection_content_hash`, `read_only: const true`, `accepted: const false`

### 3.8 Registration steps (exact)

For each contract `github.<name>.v0`:

1. Add `schemas/experimental/github.<name>.v0.schema.json` with matching `$id`
   (draft 2020-12, `additionalProperties: false`, `const` pins for `schema`,
   `independence_proven`, `review_basis_moved`, `read_only`, `accepted`).
2. Add `schemas/experimental/github.<name>.v0.example.json`. The examples for
   the record contracts are the adapter's own PR-101 pilot outputs (§9), so an
   example can never drift from what the Rust owner produces.
3. Append to `contracts.v0.json` `contracts` (kept sorted like existing
   entries), e.g.:

   ```json
   {"id":"casegraphen.experimental.github.pr_observation.v0",
    "schema_file":"github.pr_observation.v0.schema.json",
    "rust_owner":"src/github_evidence/model.rs::GITHUB_PR_OBSERVATION_SCHEMA",
    "kind":"record",
    "examples":["github.pr_observation.v0.example.json"],
    "references":["casegraphen.experimental.github.capture_manifest.v0",
                  "casegraphen.experimental.memory.source_record.v0"]}
   ```

   Kinds: `capture_manifest` is `input`; the other six are `record`.
   References: `check_evidence`/`review_finding` → `pr_observation`;
   `review_independence` → `pr_observation`, `check_evidence`,
   `review_finding`; `refresh_result` → `pr_observation`;
   `review_projection` → `pr_observation`, `check_evidence`,
   `review_finding`, `review_independence`.
4. Add a `roundtrip::<T>("github.<name>.v0.example.json")` line per contract in
   `tests/experimental_schema_conformance.rs`.
5. Add the surface paragraph to `schemas/experimental/README.md` (observations
   never become accepted facts; independence is a class, not a proof; refresh
   never rebases).
6. `python3 scripts/experimental-schema-conformance.py --check --self-test`
   must pass — it enforces inventory/example/`$ref`/`rust_owner` integrity.

## 4. Module layout

New module `src/github_evidence/` (`pub mod github_evidence;` in `lib.rs`):

| File | Contents | Why it exists |
|---|---|---|
| `mod.rs` | module doc (trust boundary, store-free invariant), re-exports | convention (`src/memory/mod.rs`) |
| `model.rs` | 7 schema constants + typed records; `deny_unknown_fields` on `CaptureManifest`; provider-mirror structs tolerant like `GitHubIssue` | single Rust owner for the contract family |
| `normalize.rs` | manifest+bytes → source records + `pr_observation` + `check_evidence[]` + `review_finding[]`; all ordering and hashing | the determinism surface, one place |
| `independence.rs` | `implementation_actor_ids`, `classify_evidence_role`, `evaluate_independence` | **single implementation** of the independence decision rule; isolated the way `evidence_trust.rs` is |
| `refresh.rs` | `classify_refresh(previous, current) -> RefreshResult` | single implementation of the stale-head rule |
| `projection.rs` | `project_review(...) -> ReviewProjection` + tier rule + loss declaration | single implementation of the tiering rule |

New single-implementation rules to add to the
`invariant-duplication-auditor` table:

| Question | Only implementation |
|---|---|
| What is the implementation actor-id set of a PR observation? | `github_evidence::independence::implementation_actor_ids` |
| What evidence role does a GitHub observation have? | `github_evidence::independence::classify_evidence_role` |
| Can this observation set satisfy an independent-review requirement? | `github_evidence::independence::evaluate_independence` |
| Is a refresh stale? | `github_evidence::refresh::classify_refresh` |
| Which review tier does a changed file/finding land in? | `github_evidence::projection` tier rule |

Explicit delegations (rules this module must **not** re-implement):

- hashing → `crate::native_hash::sha256_hex` / `content_matches_sha256`
- source-record validity → `crate::memory::{SourceRecord, validate_memory_source_record}`
- authority ceiling of tool observations → `memory::authority::origin_ceiling`
  (via `authority_origin: "tool"`; the adapter writes no authority field of
  its own)
- timestamp shape for `captured_at` → `memory::temporal::validate_timestamp`
  (export `pub(crate)` from `memory`; provider timestamps are otherwise
  verbatim strings and not validated)
- evidence acceptability → `evidence_trust::evidence_is_acceptable`, reached
  only through the existing attach/review commands, never called here
- path confinement for `artifact_path` → the canonicalized-root containment
  rule as implemented for packet artifacts (`ops/packet.rs`); extract the
  shared predicate rather than copying it

## 5. Determinism and byte-equivalent replay

Inputs to every computation: the manifest bytes and the captured artifact
bytes. Nothing else — no `SystemTime::now()`, no environment, no store, no
network. `captured_at` comes from the manifest, so replaying the same
retained files reproduces the same records byte-for-byte.

- **Canonical serialization**: typed structs serialize in declaration order;
  `serde_json`'s `Map` is a `BTreeMap` (no `preserve_order` feature), so any
  `Value` maps have sorted keys. This is the same canonical form
  `memory::validation::projection_content_hash` and
  `native_hash::case_space_checksum` already rely on.
- **Content hashes**: `sha256:<hex>` of `serde_json::to_vec(&record)` with the
  record's own hash field cleared first — the exact
  `projection_content_hash` pattern.
- **Ordering rules** (stated once in `normalize.rs`): `changed_files` by
  `path`; `check_evidence` by `(kind, name, completed_at, details_url)`;
  `review_finding` by `(authored_at, url)`; `source_record_ids`,
  `implementation_actor_ids`, and every id list lexicographic; findings and
  classifications by `subject_id`.
- **IDs are content-derived** (URL hash, head SHA, repo#pr), never counters or
  UUIDs.
- **Replay tests**: (a) run `github observe`/`github project` twice on the
  pilot and byte-compare; (b) delete all outputs and rebuild from
  `docs/pilots/issue-102/source/` + manifest, compare
  `normalized_content_hash` and `projection_content_hash` against retained
  values (exit-evidence bullet 3).

## 6. The trust boundary: how caller-declared trust dies at the door

Four mechanisms, layered:

1. **Input shape.** The only caller-authored input is the capture manifest,
   which has no trust vocabulary and `deny_unknown_fields`. `--manifest` plus
   raw bytes is the entire input surface; there is no flag that supplies a
   role, an approval, or an independence file. The six record contracts are
   outputs only, with one narrow, checked exception: `github refresh
   --previous-observation` reads a `pr_observation` record back as the
   operator's **declared review basis**. On load its
   `normalized_content_hash` is recomputed (hash field cleared, §5) and a
   mismatch is a hard refusal, so a tampered basis (e.g. head SHA edited to
   match the new capture and dodge `stale_head`) is caught. Be precise about
   what that proves: the record is *self-consistent*, not that this tool
   produced it from real provider data — a fully self-consistent forgery
   remains possible, and the mitigation is that the operator chooses which
   basis to declare. (Adversarial fixture: tampered previous observation.)
2. **Allowlist reads of provider mirrors.** Normalization maps an explicit
   allowlist of provider fields (the ones in §3). A `"trusted": true` or
   `"approved": true` planted inside captured `gh` JSON is never read — there
   is no code path that maps it — and lands in the standing
   `provider_fields_unmapped` loss declaration. (Adversarial fixture:
   caller-declared approval.)
3. **Computed actor sets and the provider's own actor discriminator.**
   `implementation_actor_ids = {pr.author.id} ∪ {commit.author.user.id} ∪
   {commit.committer.user.id}` over the captured `pr` + `commits` artifacts,
   sorted — derived by comparing actor identities **within the same
   snapshot**, never from a caller-supplied flag or list. Identity is the
   GitHub **node id** (`MDQ6VXNlcj…`, `BOT_kgDO…`), not the login: logins
   are renameable, so login matching would let a rename between the PR
   capture and a review capture silently move an actor out of the
   implementation set and into candidacy. Logins are carried for display
   only. An artifact that must feed this set but does not carry node ids
   is not usable — **hard refusal**, no login fallback (finding authors
   are handled differently; see the refusal-vs-`unattributed` split
   below).

   The retained corpus itself proves node-id keying is not hypothetical:
   the same actor appears under **two different logins** — thread-comment
   author `coderabbitai` and thread resolver `coderabbitai[bot]`, both
   `id: BOT_kgDOCCSy2w`. A login-keyed set or collapse key would treat one
   actor as two on the real dogfood data.

   Bot identity is a **provider attestation drawn from an ordered, closed
   list** — never a name heuristic:

   1. the GraphQL Actor discriminator `__typename` is not `"User"`
      (`"Bot"` for coderabbitai's reviews; `Organization`, `Mannequin`,
      `EnterpriseUserAccount` likewise fail closed into the non-human
      role);
   2. the node-id prefix `BOT_` — itself provider-issued. This outranks a
      `User` typename: the corpus's `resolvedBy` field is GraphQL-typed
      `User` and yet carries `id: BOT_kgDOCCSy2w`, so a `User`-typed
      record can still be an attested bot;
   3. id equality with an actor already bot-attested elsewhere in the same
      capture.

   Rules 2 and 3 are **load-bearing on the real corpus, not defensive
   extras**: the provider attests the same node id `BOT_kgDOCCSy2w` as
   `Bot` on its comments and `User` in `resolvedBy` within one capture, so
   per-occurrence typename attestation is self-contradictory on real data;
   only the id-keyed rules resolve it, and a bot attestation is sticky —
   a `User` typename never overrides it. The `basis` field records which
   rule fired, which is the audit trail for exactly this disagreement.

   Absent **all** of these, the actor is `unattributed` — absence is
   recorded, not guessed, and `unattributed` can never satisfy the policy,
   so the unknown fails closed. Classification is total and closed, in
   this arm order:

   ```
   1. subject is a check_run or status_context       → ci_check          (basis: check_observation)
   2. author.id ∈ implementation_actor_ids           → self_review       (basis: author_in_implementation_actor_set)
   3. bot-attested by the ordered list above         → automated_bot     (basis: provider_bot_discriminator |
                                                                                  provider_bot_id_prefix |
                                                                                  provider_bot_id_equality)
   4. typename == "User" and not bot-attested        → independent_human_candidate (basis: provider_user_discriminator)
   5. no typename and not bot-attested               → unattributed      (basis: attestation_absent)
   ```

   Refusal versus `unattributed` splits by what the field feeds. The
   artifacts the **actor-id set** is built from (`pr`, `commits`) must
   carry node ids — without them no membership test exists, so that
   capture is refused (integrity class), never patched with login
   matching. A **finding author** missing its discriminator/id classifies
   `unattributed` instead: it cannot be laundered upward (arm 5 satisfies
   nothing), and refusing the whole capture would let one malformed
   comment blind the observation. A finding author without an id also
   never collapses with anything — `duplicate_count` merging requires id
   equality, so counts are preserved conservatively rather than merged by
   name.

   Order matters twice. The implementation-actor test (arm 2) precedes the
   attestation arms, so an implementation actor can never reach
   `independent_human_candidate` — and `authorAssociation` is **not an
   input** to any arm: `MEMBER` (the PR-101 self-reviewer) confers nothing,
   `NONE` (the bot) costs nothing.

   `evaluate_independence` counts **only** `independent_human_candidate`
   subjects, and only with verbatim `review_state == "APPROVED"` whose
   `commit_sha` equals the observed head (§3.5 — no absent-binding
   fallback). Self-review, bot findings, CI success,
   implementation-authored comments, and unattributed actors are therefore
   *type-unable* to satisfy `require_independent_review`; no configuration
   reopens it. (Acceptance criteria 5 and 6. Actor-substitution fixture:
   an approval by an actor whose node id matches a commit author still
   classifies `self_review` even after a login rename — the corpus's own
   `coderabbitai`/`coderabbitai[bot]` pair is the shape of that attack.
   Association fixture: two otherwise identical approvals differing only
   in `authorAssociation` produce identical results.)

   On the PR-101 pilot this yields: all `rizumita` reviews and thread
   replies → `self_review` (arm 2 — the PR author id equals the commit
   author id); all `coderabbitai` reviews and thread comments →
   `automated_bot` (arm 3, `__typename: "Bot"`); the `coderabbitai[bot]`
   resolver identity → bot-attested via the `BOT_` prefix and id equality
   despite its `User` typename; checks → `ci_check`. Had CodeRabbit
   submitted `APPROVED` instead of `COMMENTED`, it would still classify
   `automated_bot` and could not satisfy the policy — the classifier, not
   the accident of review states, carries that guarantee. The
   `unattributed` arm is exercised by a fixture built from the retained
   *unattested* review bytes (the `reviews` section inside `pr-101.json`,
   whose author objects are `{"login": …}` only — real provider bytes, not
   a synthetic mutation).
4. **No proof inflation.** `independence_proven` is `const: false` in the
   schema and hardcoded false in Rust, with the
   `independent_minds_not_observable` finding always attached — the same
   stance `verification_policy.rs` takes. A candidate is a candidate;
   proving independent minds stays out of scope exactly as ADR-recorded for
   verification lineage.

Authority laundering is closed at a second seam: every raw artifact enters as
`memory.source_record.v0` with `authority_origin: "tool"`, so the existing
`origin_ceiling` caps anything derived from GitHub at `observation` authority.

## 7. Refresh and the stale-head rule

`classify_refresh(previous: &PrObservation, current_capture)`:

- **Basis integrity first**: the supplied previous observation's
  `normalized_content_hash` is recomputed (hash field cleared) and a
  mismatch is a hard refusal — see §6.1 for exactly what this does and does
  not prove.
- same repo/PR, `observed_head_sha == previous.head.sha` →
  `head_unchanged`, plus `observation_changes` for same-head drift:
  a check id present before and absent now (`removed` — disappearing checks),
  an existing `finding_id` whose `body_content_hash` changed (`changed` —
  edited review comment), thread resolution flips, mergeability changes.
- different head (or base, or repo/PR mismatch) → `stale_head`. The CLI
  reports it as a **domain finding** (successful result carrying an
  obstruction, exit path identical to `memory check` invalid): the
  refresh_result is emitted, `refreshed_observation_hash` is absent, and no
  new observation exists. Moving the basis requires the operator to run
  `github observe` on the new capture, which mints a visibly different
  `observation_id` and hash — never silent (acceptance criterion 4).

Cross-repository references: a manifest `repository` disagreeing with the
repository inside any captured artifact (PR url, thread urls) is a domain
finding `cross_repository_reference`, and the foreign items are excluded and
declared in `losses` — not silently included, not silently dropped. Both
channels carry the fact: `result.domain_findings` on the command envelope
(`github observe`/`github project`, same channel every other obstruction
surfaces through), and a `cross_repository_excluded` entry in
`review_projection.v0.losses` (`omitted_refs` carries the excluded URLs) on
the projection record itself. The second channel is not redundant with the
first: `review_projection.v0` is a standalone artifact — it carries its own
`projection_content_hash`, `full_trace`, `read_only`, `accepted`, and is
meant to be written out and handed to a reviewer without the command
envelope around it — so a consumer holding only the record must still be
able to see that a finding was dropped and why. `normalize.rs` remains the
single implementation of *which* findings are cross-repository;
`projection.rs` only reads that structured exclusion list back out, never
re-derives it from raw URLs.

## 8. Compact projection

Deterministic tier rule (v0, intentionally coarse):

- **Must Review**: files/subjects with unresolved actionable findings; failed
  checks; anything referenced by a `stale_head` refresh supplied to
  projection; unsatisfied `require_independent_review` policy (projected as a
  blocking finding when the flag is set).
- **Should Review**: files/subjects whose actionable findings are resolved
  (the recorded resolution itself deserves eyes), findings with
  `edited: true`, inconclusive checks (§3.7 — skipped/neutral/cancelled/
  still-running verifications that produced no evidence), and every
  verification claim whose only sources are `self_review`/`automated_bot`.
- **Can Skim**: remaining changed files with no findings.

Blocking = unresolved actionable findings + failed checks (+ unmet
independent-review requirement when demanded). Non-blocking = resolved
actionable findings, bot summaries, inconclusive checks, residual risks
(`checks_inconclusive` among them when any check is inconclusive).

Declared loss (`losses`, never empty in v0):

- `bodies_hashed`: issue/PR/comment bodies appear only as content hashes;
  full text lives in the retained source records (`full_trace`).
- `provider_fields_unmapped`: unmapped provider fields.
- `threads_truncated` / `files_truncated`: only if a capture hit a provider
  page limit — recorded from the captured `totalCount` vs node count, so a
  gluing failure is visible, not hidden.
- `cross_repository_excluded`: only if `normalize.rs` excluded one or more
  findings for naming a different repository than the manifest declares
  (§7) — `omitted_refs` carries the excluded URLs, so the projection record
  states on its own what it dropped, not only the command envelope's
  `result.domain_findings`.

The full audit trace stays separately available: `full_trace` lists every
source record id and normalized record id, and `github observe` re-emits the
complete normalized graph from the same inputs at any time. A compact
projection therefore *cites* the trace; it never replaces it.

## 9. CLI surface

Three subcommands under a new `github` command group — all read-only,
store-free, `--format json` (+ `--output`), following `memory`'s parser/ops
split (`src/native_cli/parser.rs::parse_memory`, `src/native_cli/ops/memory.rs`):

```
casegraphen github observe --manifest <github.capture_manifest.v0.json> --capture-dir <dir> --format json [--output <path>]
casegraphen github refresh --manifest <new-manifest.json> --capture-dir <new-dir> --previous-manifest <old-manifest.json> --previous-capture-dir <old-dir> [--previous-observation <github.pr_observation.v0.json>] --format json [--output <path>]
casegraphen github project --manifest <github.capture_manifest.v0.json> --capture-dir <dir> [--require-independent-review] --format json [--output <path>]
```

- `observe` → `{source_records: [...], pr_observation, check_evidence: [...],
  review_findings: [...], independence, accepted: false,
  mutation_performed: false}`. Manifest hash mismatch and path escape are
  hard errors (integrity class); cross-repo references and provider
  truncations are domain findings.
- `refresh` → `{refresh_result, accepted: false, mutation_performed: false}`;
  `stale_head` is a domain finding. **T5 ruling, resolving a gap this design
  sketch left open**: `PrObservation` alone carries no per-check or
  per-finding state, so drift detection (a disappearing check, an edited
  review comment, a thread-resolution flip — all listed below) needs the
  previous capture's normalized `check_evidence`/`review_finding` records,
  not just its `pr_observation`. The CLI therefore takes the previous review
  basis as a **capture**, not as records: `--previous-manifest`/
  `--previous-capture-dir` name the prior manifest and capture directory,
  normalized with the exact same `normalize()` every other observation goes
  through, and its resulting `check_evidence`/`review_finding` feed
  `classify_refresh` alongside its `pr_observation`. `--previous-observation`
  stays optional and is the operator's *declared* review basis (§6.1): when
  supplied, it must equal the observation re-normalized from
  `--previous-manifest`/`--previous-capture-dir` byte-for-byte, or the
  command refuses before `classify_refresh` ever runs — a declared basis
  that disagrees with the retained bytes is an integrity failure, not a
  drift. This is stronger than a bare content-hash re-verification (a fully
  self-consistent forgery, §6.1's own caveat, would still pass one) and
  needs no new record-as-input surface beyond the manifest input this family
  already has. `classify_refresh`'s own hash-recompute check (§6.1, §7)
  still runs internally as the cheaper first gate; the CLI-level equality
  check above is what actually protects a CLI caller from a tampered
  `--previous-observation` file.
- `project` → `{projection, independence, accepted: false,
  mutation_performed: false}`; with `--require-independent-review` and no
  independent human approval, the projection carries the blocking finding and
  the command reports a domain finding. `project` recomputes from the raw
  artifacts (no intermediate bundle file), which is what makes the replay
  property directly observable.

`src/cli_usage.txt` gains the three lines (with the parenthetical
"(read-only; observations are never accepted facts; no store access)"), which
automatically enrolls them in `tests/cli_surface.rs`. Integration tests spawn
the real binary via `CARGO_BIN_EXE_casegraphen` (project rule), and
`tests/product_surface.rs` gains a `github` clause proving no file in the
capture dir or elsewhere is created/modified and every output carries
`accepted: false` / `mutation_performed: false`.

No `github attach`/`accept`/`apply` exists. Follow-ups that mutate a case
space use the existing gated commands with adapter outputs as ordinary
artifacts.

## 10. Fixtures

### 10.1 Dogfood pilot `docs/pilots/issue-102/` (already retained)

`source/` contains the **real captured provider data** (captured 2026-08-06
with `gh`, re-captured the same day with the Actor `__typename`/`id`
discriminators after design review — the authoritative corpus for this work;
do not re-fetch):

| File | sha256 | Serves manifest categories | Ground truth it carries |
|---|---|---|---|
| `issue-92.json` | `e41b2147adbaf76470bba4a14ede3ed816e09dd0568b486a9690c40de1bdd355` | `issue` | issue body, state CLOSED, closed by PR 101 |
| `pr-101.json` | `07ac47fc5a0c2420ee5f5bb500001e44b6227638cfd2bc59f3e916ef2920ca26` | `pr`, `files` | base `947f347f219a60775bcf71b226ce778cc8ea21f4`, head `c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b`, `state: MERGED` with `mergeable: UNKNOWN` / `mergeStateStatus: UNKNOWN`, author `{id: MDQ6VXNlcjc5MDUxMQ==, login: rizumita}`, 78 changed files (its `reviews`/`statusCheckRollup`/`comments` sections are unmapped surplus — they lack the discriminators) |
| `pr-101-reviews.json` | `ed396eff46cbe15ee2140abf3cc916a2febecd4e0821c8b7a8f1b8752b696348` | `reviews` | GraphQL: 20 reviews, all `COMMENTED`, zero `APPROVED`; authors `{Bot, coderabbitai, BOT_kgDOCCSy2w}` ×10 and `{User, rizumita, MDQ6VXNlcjc5MDUxMQ==}` ×10; per-review `createdAt`/`lastEditedAt`; `commit.oid` on every review — 19× head `c9be9ed6…`, 1× earlier `5403673f…` (the real not-bound-to-head case for §3.5) |
| `pr-101-threads.json` | `03229662f2b7e1b327b5d8d8ef76d7e01d1fd5b883d52d6ea624dc81cff5d918` | `review_threads` | GraphQL: 9 threads, 9 resolved, `path`/`line`, per-comment author `{__typename, login, id}` + `authorAssociation` (`MEMBER` rizumita / `NONE` coderabbitai), `lastEditedAt` on 9 of 27 comments (real edited-comment data), `baseRefOid`/`headRefOid`/`mergeable` for the intra-capture consistency check — and the two-logins-one-id fact: `resolvedBy` is `{rizumita, MDQ6VXNlcj…}` on 5 threads and `{coderabbitai[bot], BOT_kgDOCCSy2w}` on 4, the same node id as comment author `coderabbitai`, GraphQL-typed `__typename: "User"` on both |
| `pr-101-commits.json` | `a597a0bc8c8682401abe0318c3750985c57570145bea493718b51cb34e8fc36b` | `commits` | GraphQL: 3 commits, author/committer `user {__typename: User, login: rizumita, id: MDQ6VXNlcjc5MDUxMQ==}` — the other half of the implementation actor-id set |
| `pr-101-checks.json` | `216cf7c74bb3a3992d89e8e195a47af9c18cb0f0ac63cdf9c439b771ceb6a82c` | `checks` | GraphQL: 2× check_run `quality`/`SUCCESS`, 1× status_context `CodeRabbit`/`SUCCESS` with description "Review rate limited" and creator `{Bot, coderabbitai, BOT_kgDOCCSy2w}` |

This corpus is **frozen**: `source/` is not written again by anyone until
T6, and T6 only adds files alongside it. The retention already carries its
own capture-replay evidence: the `reviews` and `review_threads` files were
independently re-captured by a second operator running the documented
appendix queries and reproduced **bit-for-bit** at the hashes above —
empirical support that the documented commands, not the operator, determine
the bytes (this is capture-side reproducibility; §5's normalization replay
is proven separately by the delete-and-rebuild tests).

Implementers add: `capture_manifest.v0.json` (with these hashes,
category-to-artifact mapping exactly as above, and the `gh`/`gh api graphql`
command records — retained in the implementation plan), `expected/` (retained
adapter outputs, doubling as the schema examples' source),
`comparison-report.md` (manual PR-101 review evidence vs adapter output —
acceptance criterion 12; it must state the per-occurrence-attestation
finding: on this very corpus the provider attests `BOT_kgDOCCSy2w` as `Bot`
on its comments and `User` in `resolvedBy`, so the `BOT_` id-prefix and
id-equality rules are load-bearing, not defensive extras), and `README.md`
in the `docs/pilots/issue-92/README.md` style.

The expected pilot results the integration test asserts (exit-evidence
bullet 1 and 2): two successful `quality` check evidences; nine review
threads, all resolved, zero unresolved actionable findings; exact head/base
as above; every `coderabbitai` subject classified `automated_bot` and every
`rizumita` subject `self_review`; both review `commit.oid` values normalized
verbatim (19× head, 1× `5403673f…` — the T3 unit test reuses that real
older-head review, with `review_state` swapped to `APPROVED` and a `User`
author, to prove the `approval_not_bound_to_observed_head` exclusion);
`independent_human_approvals: []`; `policy.satisfied: false` under
`--require-independent-review`; a projection with **no blocking findings**
(without the flag) whose `residual_risks` include
`no_independent_human_approval` and the rate-limited CodeRabbit status
description; and none of these observations marked accepted.

### 10.2 Adversarial fixtures `tests/fixtures/github-evidence/`

Small synthetic capture dirs (mutated copies of the pilot where convenient),
one per required case:

| Case | Fixture shape | Expected outcome |
|---|---|---|
| caller-declared approval | manifest with `"approved": true` top-level; separately, raw reviews artifact with injected `"trusted": true, "accepted": true` fields | manifest: strict-parse refusal; raw: fields unread, classification unchanged, `provider_fields_unmapped` loss |
| actor substitution | `APPROVED` review whose author node **id** equals a commit author's id but whose **login** differs (rename between captures) | `self_review` via the actor-id set — the rename does not move the actor out of the implementation set; policy unsatisfied |
| association is not independence | two otherwise identical `User` approvals at head differing only in `authorAssociation` (`MEMBER` vs `NONE`) | identical classification and policy result — `authorAssociation` is not an input |
| missing actor attestation | reviews artifact whose author objects are `{"login": …}` only — built from the retained real unattested `reviews` section of `pr-101.json`, not a synthetic mutation | authors classify `unattributed` (fail-closed arm 5); an `APPROVED` review among them satisfies nothing; finding counts preserved (no id ⇒ no collapse) |
| actor-set source without ids | `commits` (or `pr`) artifact whose user objects lack node ids | **hard refusal**, integrity class — the implementation actor set cannot be built, and login matching is not a fallback |
| tampered previous observation | `--previous-observation` with head SHA edited to match the new capture, `normalized_content_hash` left stale | **hard refusal** on load — hash recomputation mismatch; the stale-head dodge never reaches `classify_refresh` |
| stale head | refresh capture whose PR head differs from previous observation | `disposition: stale_head`, no `refreshed_observation_hash`, domain finding |
| disappearing checks | same head, second capture missing one `quality` check run | `observation_changes: [{category: checks, change: removed, …}]` |
| skipped check | one `quality` check with conclusion `SKIPPED` | listed in `inconclusive_checks`, lands in Should Review with `checks_inconclusive` residual risk — never silently folded into success |
| edited review comments | same head, `lastEditedAt` set and body changed on an existing finding | finding `edited: true`; refresh reports `changed` with differing `body_content_hash` (the real pilot already carries 9 edited comments for the `edited` bit itself) |
| duplicate bot findings | two `Bot` comments with identical `(author.id, body, path)` | one finding, `duplicate_count: 2`; actionable count preserved |
| cross-repository references | thread URL pointing at another repository | `cross_repository_reference` domain finding; foreign item excluded from `review_findings` and declared in `review_projection.v0.losses` (`cross_repository_excluded`, §7) — visible in both the command envelope and the standalone projection record |

The only satisfying shape — an `APPROVED` review at the observed head by a
provider-attested `User` whose id is outside the implementation actor set —
is exercised both as a T3 unit-test variant built from the real pilot review
data (§10.1) and, per the team-lead brief for T6, as a paired CLI-level
fixture through the real binary (`positive-control-outside-approval` /
`positive-control-older-binding` under `tests/fixtures/github-evidence/`,
differing in exactly one field — the review's `commit.oid`), so the positive
path is proven alongside the twelve refusal/exclusion fixtures above and is
not provable only by a classifier that always answers "not satisfied".

## 11. Non-goals, enforced

- **GitHub is not a source of truth**: the adapter is store-free (no
  `native_store` import, no `--store` flag); its outputs are files an operator
  may later submit through the gated attach/review seam, where
  `evidence_trust` and the operation gates decide as before.
- **Nothing is auto-accepted**: every record carries `accepted: false`
  (`const` in-schema) and `mutation_performed: false`; `product_surface.rs`
  proves the commands write nothing.
- **No authority from roles/labels/bots**: `author_association`, labels, and
  bot identities are preserved verbatim but no code path reads them for
  authorization; source records enter at `tool` origin → `observation`
  ceiling.
- **No credential ingestion**: intake reads only manifest-listed artifact
  files under `--capture-dir`; `command_record` is retained data, never
  executed; no environment or header is read into any record.
- **No process routing**: no orchestration; #100's skill decides *when* to run
  these commands.

## 12. Acceptance-criteria mapping

| # | Criterion / exit evidence | Design element | Proving test |
|---|---|---|---|
| A1 | experimental contracts for observations, check evidence, review findings, independence, refresh | §3 seven contracts + §3.8 registration | `experimental_schema_conformance.rs` roundtrips + python gate |
| A2 | snapshot bound to repo, PR, base SHA, head SHA | `pr_observation.base/head/repository/pr` (§3.2) | pilot integration test asserts exact SHAs |
| A3 | URLs, timestamps, actors, roles, hashes preserved without acceptance | verbatim provider strings + `accepted: const false` (§3, §5) | schema `const` + `product_surface.rs` github clause |
| A4 | stale-head refresh refused/reported, never silent | `refresh.rs` + `review_basis_moved: const false` + verified previous-observation hash (§6.1, §7) | stale-head + tampered-previous fixtures; unit tests on `classify_refresh` |
| A5 | self/bot/CI/independent-human as distinct roles | `evidence_role` enum (5 roles incl. fail-closed `unattributed`) + closed classifier keyed on the actor-id set and the ordered bot-attestation list (`__typename`, `BOT_` id prefix, id equality); absent attestation is recorded as `unattributed`, never guessed (§6) | classifier unit tests over all five arms; pilot exercises `self_review`/`automated_bot`/`ci_check` on real data (candidate via T3 variant, `unattributed` via the real unattested-bytes fixture) |
| A6 | self-review provably cannot satisfy independent review | classifier ordering + `evaluate_independence` counting only `User`-attested candidates with `APPROVED` bound to the exact head — no absent-binding fallback; exclusions visible in `excluded_approvals` (§3.5, §6) | actor-substitution (id-based rename) + association + older-head-approval tests; pilot asserts `satisfied: false` |
| A7 | actionable counts and thread state preserved | `duplicate_count`, `thread.resolved/outdated` (§3.4) | duplicate-bot fixture; pilot 9/9 threads |
| A8 | compact Must/Should/Can with loss + separate full trace | `review_projection.v0` tiers, `losses`, `full_trace` (§8) | projection unit tests + pilot integration test |
| A9 | byte-equivalent rebuild | §5 determinism; content-derived ids; manifest-fixed time | double-run + delete-and-rebuild replay tests |
| A10 | mutations stay behind existing gates | store-free adapter; no new mutation command (§2, §9, §11) | `product_surface.rs`; absence of any `github` mutation path in `cli_usage.txt` |
| A11 | adversarial fixtures (the issue's 7 cases + 5 added: association-not-read, missing attestation → `unattributed`, actor-set source without ids, tampered previous observation, skipped check) | §10.2 | one test per fixture in `tests/github_evidence.rs` |
| A12 | dogfood vs manual comparison retained | `docs/pilots/issue-102/comparison-report.md` (§10.1) | pilot README gate commands; report checked in |
| E1 | replay reports 2 Quality successes, 9 resolved threads, exact head/base, no independent approval — unlaundered | §10.1 expected results | pilot integration test |
| E2 | no blocking findings; rate-limited bot review and self-review limitation declared | `residual_risks` + tier rule (§8) | pilot integration test on projection content |
| E3 | delete derived outputs, rebuild → same normalized result | `project`/`observe` recompute from source (§5, §9) | delete-and-rebuild replay test |

## 13. Open contingency

ADR numbering: this branch's `main` has no tracked ADR past 0029, so this
branch legitimately owns **ADR 0030** (`docs/adr/0030-github-evidence-observation-boundary.md`);
the next available identifier after it is 0031. If issue #100's branch also
lands its own ADR 0030 before this one merges, the ADR conformance gate's
contiguous, non-duplicate identifier rule means whichever of the two merges
second must renumber to the next free slot (file, heading, links, and the
next-available counters in `docs/adr/0012` consequences and `README.md`).
`cargo test --test adr_conformance` is the oracle either way.

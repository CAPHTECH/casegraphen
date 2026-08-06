# Issue 102 dogfood: manual PR-101 review evidence vs adapter output

This report is acceptance criterion A12. It compares what a human reviewer
manually recorded about Issue #92 / PR #101 (the Memory Plane pilot, #101,
[`947f347f…`→`c9be9ed6…`](https://github.com/CAPHTECH/casegraphen/pull/101))
against what `casegraphen github observe|project` reproduces from the same
retained provider capture (`docs/pilots/issue-102/source/`, §10.1 of the
[design doc](../../design/issue-102-github-evidence-adapter.md)). The point is
not "the adapter agrees with the human" in the abstract — it is *what the
manual process recorded*, *what the adapter reproduces exactly*, and *what
only the adapter makes explicit* that the manual process could not, or did
not, state.

## What the manual dogfood recorded

PR #101 was merged after a manual review pass: two `quality` CI checks green,
a CodeRabbit automated review pass (partially rate-limited), nine review
threads opened and manually marked resolved, and a human approval recorded
informally by merging the PR. The manual record — the PR's own timeline and
the merge itself — states that review happened and concluded successfully.
It does not state, because GitHub's UI does not surface it as a fact to
check, *who* the reviewing identities were relative to the PR's authorship,
nor that the automated review was rate-limited on some passes rather than
fully evaluating the diff.

## What the adapter reproduces exactly

Running `casegraphen github observe` against the retained capture reproduces
the manual record's own facts, with nothing rounded off or laundered into
acceptance:

| Fact | Manual record | Adapter (`docs/pilots/issue-102/expected/`) |
|---|---|---|
| Base / head SHA | `947f347f…` / `c9be9ed6…` | `pr_observation.base.sha` / `.head.sha` — identical |
| Liveness | Merged | `liveness.state: "MERGED"`, `liveness.mergeable: "UNKNOWN"` (GitHub stops reporting mergeability post-merge; carried verbatim, not coerced to a boolean) |
| Changed files | 78 files touched | `changed_files.len() == 78` |
| CI checks | Two `quality` checks green | `check_evidence`: two `check_run/quality/SUCCESS` |
| CodeRabbit review | Ran, partially rate-limited | one `status_context/CodeRabbit/SUCCESS` with `description: "Review rate limited"`, surfaced in `review_projection.residual_risks` as `status_context_description` — not dropped as an unmapped provider field |
| Review threads | Nine opened, nine resolved | `review_findings`: nine distinct `thread_id`s, all `thread.resolved: true`; `review_projection.unresolved_threads: []` |
| Merge decision | Merged (implicit "reviewed") | `review_projection.blocking_findings: []` — the compact projection agrees nothing was left unresolved |

The `review_projection.v0` also carries what the manual dogfood's merge event
never distinguished: two residual risks survive into the compact
projection even with zero blocking findings —
`no_independent_human_approval` and the rate-limited CodeRabbit description
— so a projection that "reads clean" (no `blocking_findings`) still
visibly declares the two things worth a reviewer's attention that a bare
"merged" status does not.

## What only the adapter makes explicit

Two facts the manual dogfood process never recorded, because nothing in
GitHub's PR UI computes them, are exactly what this adapter exists to make
explicit and unlaunderable:

**Every human review on PR #101 is self-review.** `rizumita`
(`MDQ6VXNlcjc5MDUxMQ==`) is the PR author, every commit author, and every
commit committer on this PR — `pr_observation.implementation_actors.actor_ids
== ["MDQ6VXNlcjc5MDUxMQ=="]`. Every `rizumita` review and thread reply
therefore classifies `self_review` via `author_in_implementation_actor_set`,
*despite* GitHub's own `authorAssociation: MEMBER` on those reviews —
`authorAssociation` is never read by the classifier (design §6). The manual
merge event recorded "reviewed and merged"; it could not and did not record
that no reviewing identity on this PR was independent of its author. The
adapter's own `review_independence.v0` record states it as a fact:
`independent_human_approvals: []`, `policy.satisfied: false` under
`--require-independent-review`, and the standing
`independent_minds_not_observable` finding attached regardless (the
classifier never claims to *prove* independence — it only rules out the one
shape that provably cannot supply it).

**The provider contradicts itself about actor type on this exact corpus.**
The node id `BOT_kgDOCCSy2w` (CodeRabbit) is attested `__typename: "Bot"` on
its own review and thread-comment authorship, and `__typename: "User"` on
`resolvedBy` for four of the nine review threads (GitHub's `reviewThreads`
query types `resolvedBy` as `User` unconditionally — an `... on Bot { id }`
fragment there is a hard GraphQL query error, so no query can ask for a
better-typed answer). The same actor further appears under two different
logins in this one capture: `coderabbitai` (review/comment author) and
`coderabbitai[bot]` (thread resolver) — both `id: BOT_kgDOCCSy2w`. A manual
reviewer reading the GitHub UI sees two login strings and, if checking at
all, two typenames; nothing prompts noticing they name one actor. This is
exactly why per-occurrence `__typename` attestation and login-keyed identity
are both unsafe as implemented here, not as a hypothetical defensive
extra — the ordered bot-attestation list (design §6: typename, then the
provider-issued `BOT_` id prefix, then id-equality with an actor already
bot-attested elsewhere in the capture) is load-bearing on this very PR. The
adapter's `implementation_actors`/classifier keys every identity by GitHub
node id for exactly this reason, and `review_finding.v0`'s `thread.resolved_by`
records both the id and the login side by side so the discrepancy stays
visible in the retained data rather than being resolved silently.

## Capture-side reproducibility

Separately from the normalization replay above (§5's delete-and-rebuild
property, proven by `rebuild_from_retained_source_matches_retained_expected_hashes`
in `tests/github_evidence.rs`), the two GraphQL capture queries in the
design doc's Appendix (`reviews`, `review_threads`) were independently
re-run against the live GitHub API by a second operator after design review
and reproduced their §10.1 SHA-256 hashes byte-for-byte. That is evidence
the *documented capture commands*, not the operator who ran them, determine
the bytes — a different, earlier link in the reproducibility chain than the
adapter's own normalization determinism, which only starts once bytes are
already on disk.

## Conclusion

The adapter does not disagree with the manual dogfood record on any fact the
manual record actually stated. It reproduces every one of them exactly
(head/base, file count, check outcomes, thread resolution) without rounding
a `MERGED`+`UNKNOWN`-mergeable PR into a boolean, and without folding the
rate-limited CodeRabbit pass into a plain "success". What it adds is
strictly additive: a computed, id-keyed independence classification the
manual process had no mechanism to compute, and a visible record of a
provider self-contradiction (`Bot` vs `User` on the same node id) that a
human skimming logins would not notice. Both are surfaced as declared,
inspectable records (`review_independence.v0`, `review_finding.v0`), never
as a silently upgraded acceptance status — `accepted: false` on every record
this adapter emits, unconditionally.

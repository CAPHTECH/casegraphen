#!/usr/bin/env python3
"""Minimal synthetic capture fixtures for the design §10.2 cases that don't
need real pilot bytes: actor substitution, association-not-independence,
actor-set-source-without-ids, disappearing checks, skipped check, edited
review comments, duplicate bot findings, cross-repository references.

Same minimal sextet shape `tests/github_evidence.rs`'s own
`write_stale_head_previous_capture` already uses for the stale-head case,
kept identical here for consistency.

Run from anywhere: `python3 tests/fixtures/github-evidence/generate_synthetic.py`.
Determinism (design doc §5) means re-running this reproduces the same bytes
committed here; it is not part of the test or build gate.
"""
import hashlib
import json
import os

FIXTURES_ROOT = os.path.dirname(os.path.abspath(__file__))

REPO = "OWNER/repo"
PR_NUMBER = 7
BASE_SHA = "a" * 40
HEAD_SHA = "b" * 40
PR_AUTHOR_ID = "actor:pr-author"
PR_AUTHOR_LOGIN = "alice"
COMMITTER_ID = "actor:committer"
COMMITTER_LOGIN = "old-login"


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write_json(path: str, value) -> bytes:
    data = json.dumps(value, ensure_ascii=False, indent=2).encode("utf-8") + b"\n"
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as fh:
        fh.write(data)
    return data


def base_pr(head_sha=HEAD_SHA, base_sha=BASE_SHA, state="OPEN",
            mergeable="MERGEABLE", merge_state_status="CLEAN"):
    return {
        "number": PR_NUMBER, "title": "Add a thing",
        "url": f"https://github.com/{REPO}/pull/{PR_NUMBER}",
        "state": state,
        "author": {"id": PR_AUTHOR_ID, "login": PR_AUTHOR_LOGIN},
        "baseRefName": "main", "baseRefOid": base_sha,
        "headRefName": "feature", "headRefOid": head_sha,
        "createdAt": "2026-01-01T00:00:00Z", "mergedAt": None,
        "mergeable": mergeable, "mergeStateStatus": merge_state_status,
        "body": "pr body",
    }


def base_files():
    return {"files": [
        {"path": "a.rs", "additions": 1, "deletions": 0, "changeType": "ADDED"},
        {"path": "b.rs", "additions": 2, "deletions": 1, "changeType": "MODIFIED"},
    ]}


def base_reviews(nodes=None):
    nodes = nodes or []
    return {"data": {"repository": {"pullRequest": {
        "number": PR_NUMBER,
        "reviews": {"totalCount": len(nodes), "nodes": nodes},
    }}}}


def base_threads(head_sha=HEAD_SHA, base_sha=BASE_SHA, nodes=None):
    nodes = nodes or []
    return {"data": {"repository": {"pullRequest": {
        "number": PR_NUMBER, "baseRefOid": base_sha, "headRefOid": head_sha,
        "reviewThreads": {"totalCount": len(nodes), "nodes": nodes},
    }}}}


def base_commits(author_id=PR_AUTHOR_ID, author_login=PR_AUTHOR_LOGIN,
                  committer_id=COMMITTER_ID, committer_login=COMMITTER_LOGIN,
                  author_has_id=True, committer_has_id=True):
    def user(id_, login, has_id):
        u = {"__typename": "User", "login": login}
        if has_id:
            u["id"] = id_
        return u
    return {"data": {"repository": {"pullRequest": {"commits": {"totalCount": 1, "nodes": [
        {"commit": {
            "author": {"user": user(author_id, author_login, author_has_id)},
            "committer": {"user": user(committer_id, committer_login, committer_has_id)},
        }}
    ]}}}}}


def commits_value(nodes, total_count=None):
    """A `commits` capture with an explicit node list — used by the S2
    truncated-commits fixture, where `total_count` must be able to disagree
    with `len(nodes)`."""
    if total_count is None:
        total_count = len(nodes)
    return {"data": {"repository": {"pullRequest": {
        "commits": {"totalCount": total_count, "nodes": nodes},
    }}}}


def commit_node(actor_id, login):
    user = {"__typename": "User", "login": login, "id": actor_id}
    return {"commit": {"author": {"user": user}, "committer": {"user": user}}}


def base_checks(head_sha=HEAD_SHA, contexts=None, contexts_total_count=None):
    contexts = contexts if contexts is not None else [{
        "__typename": "CheckRun", "name": "quality", "status": "COMPLETED",
        "conclusion": "SUCCESS", "startedAt": "2026-01-01T03:00:00Z",
        "completedAt": "2026-01-01T03:05:00Z",
        "detailsUrl": "https://ci.example/1",
        "checkSuite": {"app": {"slug": "github-actions"},
                        "workflowRun": {"workflow": {"name": "Quality"}}},
    }]
    if contexts_total_count is None:
        contexts_total_count = len(contexts)
    return {"data": {"repository": {"pullRequest": {
        "headRefOid": head_sha,
        "commits": {"nodes": [{"commit": {
            "oid": head_sha,
            "statusCheckRollup": {"contexts": {"totalCount": contexts_total_count, "nodes": contexts}},
        }}]},
    }}}}


def write_capture(fixture_dir, pr, files, reviews, threads, commits, checks,
                   repo=REPO, pr_number=PR_NUMBER, issue_numbers=None,
                   issue=None, issue_number=None):
    issue_numbers = issue_numbers or []
    entries_spec = [
        ("pr", "pr.json", pr),
        ("files", "files.json", files),
        ("reviews", "reviews.json", reviews),
        ("review_threads", "review_threads.json", threads),
        ("commits", "commits.json", commits),
        ("checks", "checks.json", checks),
    ]
    entries = []
    for category, filename, value in entries_spec:
        path = os.path.join(fixture_dir, filename)
        data = write_json(path, value)
        entries.append({
            "category": category, "artifact_path": filename,
            "content_hash": f"sha256:{sha256_hex(data)}", "command_record": [],
        })
    if issue is not None:
        path = os.path.join(fixture_dir, "issue.json")
        data = write_json(path, issue)
        entries.append({
            "category": "issue", "issue_number": issue_number,
            "artifact_path": "issue.json",
            "content_hash": f"sha256:{sha256_hex(data)}", "command_record": [],
        })
    manifest = {
        "schema": "casegraphen.experimental.github.capture_manifest.v0",
        "repository": repo, "issue_numbers": issue_numbers, "pr_number": pr_number,
        "captured_at": "2026-01-01T00:00:00Z", "capture_tool": "gh",
        "entries": entries,
    }
    write_json(os.path.join(fixture_dir, "manifest.json"), manifest)


def actor_object(typename, login, id_=None):
    obj = {"login": login}
    if typename is not None:
        obj["__typename"] = typename
    if id_ is not None:
        obj["id"] = id_
    return obj


def review_node(node_id, state, author, commit_sha=HEAD_SHA, body="",
                 association="NONE", submitted_at="2026-01-01T02:00:00Z"):
    return {
        "state": state, "body": body, "submittedAt": submitted_at,
        "lastEditedAt": None,
        "url": f"https://github.com/{REPO}/pull/{PR_NUMBER}#{node_id}",
        "authorAssociation": association,
        "commit": {"oid": commit_sha}, "author": author,
    }


def thread_comment_node(comment_id, author, body="please fix",
                         created_at="2026-01-01T02:30:00Z", last_edited_at=None,
                         url=None, association="NONE"):
    return {
        "body": body, "createdAt": created_at, "lastEditedAt": last_edited_at,
        "url": url or f"https://github.com/{REPO}/pull/{PR_NUMBER}#{comment_id}",
        "authorAssociation": association, "author": author,
    }


def thread_node(thread_id, comments, resolved=True, outdated=False, path="a.rs",
                 resolved_by=None, comments_total_count=None):
    if comments_total_count is None:
        comments_total_count = len(comments)
    return {
        "id": thread_id, "isResolved": resolved, "isOutdated": outdated,
        "path": path, "resolvedBy": resolved_by,
        "comments": {"totalCount": comments_total_count, "nodes": comments},
    }


# ---------------------------------------------------------------------
# actor substitution: an APPROVED review author id equals a commit
# author's node id, but the review's login differs (a rename between
# captures). self_review must still fire on id equality.
# ---------------------------------------------------------------------
def build_actor_substitution():
    fixture_dir = os.path.join(FIXTURES_ROOT, "actor-substitution")
    reviewer = actor_object("User", "renamed-login", COMMITTER_ID)  # same id, new login
    reviews = base_reviews([review_node("pullrequestreview-1", "APPROVED", reviewer)])
    write_capture(
        fixture_dir, base_pr(), base_files(), reviews, base_threads(),
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# association is not independence: two otherwise-identical outside
# APPROVED reviews differing only in authorAssociation.
# ---------------------------------------------------------------------
def build_association_not_independence():
    fixture_dir = os.path.join(FIXTURES_ROOT, "association-not-independence")
    outside_member = actor_object("User", "carol", "actor:outside-member")
    outside_none = actor_object("User", "dave", "actor:outside-none")
    reviews = base_reviews([
        review_node("pullrequestreview-1", "APPROVED", outside_member, association="MEMBER"),
        review_node("pullrequestreview-2", "APPROVED", outside_none, association="NONE"),
    ])
    write_capture(
        fixture_dir, base_pr(), base_files(), reviews, base_threads(),
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# actor-set source without ids: the commits artifact's user objects lack
# node ids -> hard refusal (the implementation actor set cannot be built).
# ---------------------------------------------------------------------
def build_actor_set_source_without_ids():
    fixture_dir = os.path.join(FIXTURES_ROOT, "actor-set-source-without-ids")
    commits = base_commits(author_has_id=False)
    write_capture(
        fixture_dir, base_pr(), base_files(), base_reviews(), base_threads(),
        commits, base_checks(),
    )


# ---------------------------------------------------------------------
# skipped check: one check with conclusion SKIPPED -> inconclusive_checks.
# ---------------------------------------------------------------------
def build_skipped_check():
    fixture_dir = os.path.join(FIXTURES_ROOT, "skipped-check")
    contexts = [{
        "__typename": "CheckRun", "name": "quality", "status": "COMPLETED",
        "conclusion": "SKIPPED", "startedAt": "2026-01-01T03:00:00Z",
        "completedAt": "2026-01-01T03:05:00Z",
        "detailsUrl": "https://ci.example/1",
        "checkSuite": {"app": {"slug": "github-actions"},
                        "workflowRun": {"workflow": {"name": "Quality"}}},
    }]
    write_capture(
        fixture_dir, base_pr(), base_files(), base_reviews(), base_threads(),
        base_commits(), base_checks(contexts=contexts),
    )


# ---------------------------------------------------------------------
# duplicate bot findings: two identical Bot comments (same author id,
# body, path) in one thread -> one finding, duplicate_count: 2.
# ---------------------------------------------------------------------
def build_duplicate_bot_findings():
    fixture_dir = os.path.join(FIXTURES_ROOT, "duplicate-bot-findings")
    bot = actor_object("Bot", "reviewbot", "actor:bot-1")
    comments = [
        thread_comment_node("discussion_r1", bot, body="nitpick: x", created_at="2026-01-01T02:30:00Z"),
        thread_comment_node("discussion_r2", bot, body="nitpick: x", created_at="2026-01-01T02:35:00Z"),
    ]
    threads = base_threads(nodes=[thread_node("thread-1", comments, resolved=False)])
    write_capture(
        fixture_dir, base_pr(), base_files(), base_reviews(), threads,
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# cross-repository references: a thread comment URL naming a different
# repository -> cross_repository_reference domain finding; the item is
# excluded from review_findings.
# ---------------------------------------------------------------------
def build_cross_repository_references():
    fixture_dir = os.path.join(FIXTURES_ROOT, "cross-repository-references")
    bot = actor_object("Bot", "reviewbot", "actor:bot-1")
    foreign_comment = thread_comment_node(
        "discussion_r1", bot, body="see OTHER/other#1",
        url="https://github.com/OTHER/other/pull/1#discussion_r1",
    )
    local_comment = thread_comment_node("discussion_r2", bot, body="local note")
    threads = base_threads(nodes=[
        thread_node("thread-1", [foreign_comment], resolved=False, path="a.rs"),
        thread_node("thread-2", [local_comment], resolved=False, path="b.rs"),
    ])
    write_capture(
        fixture_dir, base_pr(), base_files(), base_reviews(), threads,
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# disappearing checks (previous vs current, same head): current capture
# is missing a check run present in the previous one.
# ---------------------------------------------------------------------
def build_disappearing_checks():
    previous_dir = os.path.join(FIXTURES_ROOT, "disappearing-checks", "previous")
    current_dir = os.path.join(FIXTURES_ROOT, "disappearing-checks", "current")
    contexts_previous = [
        {"__typename": "CheckRun", "name": "quality", "status": "COMPLETED",
         "conclusion": "SUCCESS", "startedAt": "2026-01-01T03:00:00Z",
         "completedAt": "2026-01-01T03:05:00Z", "detailsUrl": "https://ci.example/1",
         "checkSuite": {"app": {"slug": "github-actions"},
                         "workflowRun": {"workflow": {"name": "Quality"}}}},
        {"__typename": "CheckRun", "name": "lint", "status": "COMPLETED",
         "conclusion": "SUCCESS", "startedAt": "2026-01-01T03:00:00Z",
         "completedAt": "2026-01-01T03:05:00Z", "detailsUrl": "https://ci.example/2",
         "checkSuite": {"app": {"slug": "github-actions"},
                         "workflowRun": {"workflow": {"name": "Lint"}}}},
    ]
    contexts_current = contexts_previous[:1]  # `lint` disappeared
    write_capture(
        previous_dir, base_pr(), base_files(), base_reviews(), base_threads(),
        base_commits(), base_checks(contexts=contexts_previous),
    )
    write_capture(
        current_dir, base_pr(), base_files(), base_reviews(), base_threads(),
        base_commits(), base_checks(contexts=contexts_current),
    )


# ---------------------------------------------------------------------
# edited review comments (previous vs current, same head): the second
# capture edits an existing thread comment's body (lastEditedAt set).
# ---------------------------------------------------------------------
def build_edited_review_comments():
    previous_dir = os.path.join(FIXTURES_ROOT, "edited-review-comments", "previous")
    current_dir = os.path.join(FIXTURES_ROOT, "edited-review-comments", "current")
    bot = actor_object("Bot", "reviewbot", "actor:bot-1")
    previous_comment = thread_comment_node(
        "discussion_r1", bot, body="original text", created_at="2026-01-01T02:30:00Z",
    )
    current_comment = thread_comment_node(
        "discussion_r1", bot, body="edited text", created_at="2026-01-01T02:30:00Z",
        last_edited_at="2026-01-01T04:00:00Z",
    )
    previous_threads = base_threads(nodes=[
        thread_node("thread-1", [previous_comment], resolved=False)
    ])
    current_threads = base_threads(nodes=[
        thread_node("thread-1", [current_comment], resolved=False)
    ])
    write_capture(
        previous_dir, base_pr(), base_files(), base_reviews(), previous_threads,
        base_commits(), base_checks(),
    )
    write_capture(
        current_dir, base_pr(), base_files(), base_reviews(), current_threads,
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# colliding finding ids (S1): two review nodes share one URL, so
# `finding_id = sha256(url)` collides — a self-review APPROVED-at-head
# review and an outside COMMENTED review under the same finding_id. A
# provider impossibility (a review's URL is its own permalink); the capture
# must be refused, never normalized by picking a winner.
# ---------------------------------------------------------------------
def build_colliding_finding_ids():
    fixture_dir = os.path.join(FIXTURES_ROOT, "colliding-finding-ids")
    self_reviewer = actor_object("User", PR_AUTHOR_LOGIN, PR_AUTHOR_ID)
    outsider = actor_object("User", "carol", "actor:outside")
    reviews = base_reviews([
        review_node("pullrequestreview-1", "APPROVED", self_reviewer,
                     body="shipping my own work", association="MEMBER"),
        review_node("pullrequestreview-1", "COMMENTED", outsider,
                     body="drive-by note from an outsider", association="NONE",
                     submitted_at="2026-01-01T03:00:00Z"),
    ])
    write_capture(
        fixture_dir, base_pr(), base_files(), reviews, base_threads(),
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# commits truncation (S2): a `full` capture (totalCount 2, 2 commit nodes —
# pr-author and bob) and a `truncated` capture with the identical
# totalCount but only the pr-author node. Same outside-shaped APPROVED
# review from bob in both, to pin the boundary: `full` classifies bob as
# self_review (he is an implementation actor) and does not satisfy the
# independent-review policy; `truncated` must refuse outright rather than
# silently let bob's shrunk-out-of-the-actor-set review look independent.
# ---------------------------------------------------------------------
def build_commits_truncation():
    base_dir = os.path.join(FIXTURES_ROOT, "commits-truncation")
    full_dir = os.path.join(base_dir, "full")
    truncated_dir = os.path.join(base_dir, "truncated")
    bob = actor_object("User", "bob", "actor:bob")
    reviews = base_reviews([review_node("pullrequestreview-1", "APPROVED", bob)])
    nodes = [
        commit_node(PR_AUTHOR_ID, PR_AUTHOR_LOGIN),
        commit_node("actor:bob", "bob"),
    ]
    write_capture(
        full_dir, base_pr(), base_files(), reviews, base_threads(),
        commits_value(nodes, total_count=2), base_checks(),
    )
    write_capture(
        truncated_dir, base_pr(), base_files(), reviews, base_threads(),
        commits_value(nodes[:1], total_count=2), base_checks(),
    )


# ---------------------------------------------------------------------
# single check changed (S3): same head, two checks in both captures —
# `stable-check`'s conclusion never changes, `flipping-check`'s does
# (SUCCESS -> FAILURE). Because both checks come from the same checks.json
# file, the byte change to `flipping-check` renames every check's
# `source_record_id` (derived from the whole-file hash) even though
# `stable-check` did not change — the refresh must report only
# `flipping-check`, never `stable-check`.
# ---------------------------------------------------------------------
def build_single_check_changed():
    base_dir = os.path.join(FIXTURES_ROOT, "single-check-changed")
    previous_dir = os.path.join(base_dir, "previous")
    current_dir = os.path.join(base_dir, "current")

    def contexts(flip_conclusion):
        return [
            {"__typename": "CheckRun", "name": "stable-check", "status": "COMPLETED",
             "conclusion": "SUCCESS", "startedAt": "2026-01-01T04:00:00Z",
             "completedAt": "2026-01-01T04:10:00Z", "detailsUrl": "https://ci.example/stable",
             "checkSuite": {"app": {"slug": "github-actions"},
                             "workflowRun": {"workflow": {"name": "Stable"}}}},
            {"__typename": "CheckRun", "name": "flipping-check", "status": "COMPLETED",
             "conclusion": flip_conclusion, "startedAt": "2026-01-01T04:00:00Z",
             "completedAt": "2026-01-01T04:20:00Z", "detailsUrl": "https://ci.example/flip",
             "checkSuite": {"app": {"slug": "github-actions"},
                             "workflowRun": {"workflow": {"name": "Flip"}}}},
        ]

    write_capture(
        previous_dir, base_pr(), base_files(), base_reviews(), base_threads(),
        base_commits(), base_checks(contexts=contexts("SUCCESS")),
    )
    write_capture(
        current_dir, base_pr(), base_files(), base_reviews(), base_threads(),
        base_commits(), base_checks(contexts=contexts("FAILURE")),
    )


# ---------------------------------------------------------------------
# duplicate issue number (S4): `manifest.issue_numbers` declares issue 42
# twice against a single matching `issue` entry. Caller input must not be
# able to shape a tool-computed record's cardinality this way.
# ---------------------------------------------------------------------
def build_duplicate_issue_number():
    fixture_dir = os.path.join(FIXTURES_ROOT, "duplicate-issue-number")
    issue = {
        "number": 42, "title": "Some issue", "state": "OPEN",
        "url": f"https://github.com/{REPO}/issues/42",
        "createdAt": "2026-01-01T00:00:00Z", "body": "issue body",
    }
    write_capture(
        fixture_dir, base_pr(), base_files(), base_reviews(), base_threads(),
        base_commits(), base_checks(),
        issue_numbers=[42, 42], issue=issue, issue_number=42,
    )


# ---------------------------------------------------------------------
# outside human commented review (S9): a non-actionable review summary
# (review summaries are never actionable, `normalize.rs`) from an
# independent human candidate who did not approve. Before the fix this
# reached no tier and no finding list at all.
# ---------------------------------------------------------------------
def build_outside_human_commented_review():
    fixture_dir = os.path.join(FIXTURES_ROOT, "outside-human-commented-review")
    outsider = actor_object("User", "carol", "actor:outside")
    reviews = base_reviews([review_node(
        "pullrequestreview-1", "COMMENTED", outsider,
        body="I am not convinced the gate actually holds; please justify before merging.",
        association="NONE",
    )])
    write_capture(
        fixture_dir, base_pr(), base_files(), reviews, base_threads(),
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# truncated thread (S10): `comments.totalCount: 3`, zero comment nodes
# received, on file `b.rs`. `unresolved_threads`/`can_skim`/the
# `threads_truncated` loss must all still account for it.
# ---------------------------------------------------------------------
def build_truncated_thread():
    fixture_dir = os.path.join(FIXTURES_ROOT, "truncated-thread")
    threads = base_threads(nodes=[
        thread_node("thread-truncated", [], resolved=False, path="b.rs", comments_total_count=3)
    ])
    write_capture(
        fixture_dir, base_pr(), base_files(), base_reviews(), threads,
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# empty unresolved thread (S10): `comments.totalCount: 0` — a review
# thread with no initiating comment is a provider impossibility. Hard
# refusal, not a silently invisible thread.
# ---------------------------------------------------------------------
def build_empty_unresolved_thread():
    fixture_dir = os.path.join(FIXTURES_ROOT, "empty-unresolved-thread")
    threads = base_threads(nodes=[
        thread_node("thread-ghost", [], resolved=False, path="a.rs", comments_total_count=0)
    ])
    write_capture(
        fixture_dir, base_pr(), base_files(), base_reviews(), threads,
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# collapse loses actionable (S11): a byte-identical pair (`-e`/`-f`)
# differing only in which of two duplicate thread comments' URL suffixes
# sorts first. Before the fix, the `(authored_at, url)` collapse tie-break
# on an `authored_at` tie decided `actionable` by URL order rather than by
# which occurrence was the thread's actual opener.
# ---------------------------------------------------------------------
def build_collapse_actionable_pair():
    bot = actor_object("Bot", "reviewbot", "actor:bot-1")
    same_created_at = "2026-01-01T02:30:00Z"

    def comments(opener_suffix, reply_suffix):
        return [
            thread_comment_node(opener_suffix, bot, body="dup", created_at=same_created_at),
            thread_comment_node(reply_suffix, bot, body="dup", created_at=same_created_at),
        ]

    # E: the opener's URL suffix (`discussion_r99`) sorts *after* the
    # reply's (`discussion_r100`) — `"discussion_r100" < "discussion_r99"`
    # lexicographically (`'1' < '9'` at the first differing digit) — so the
    # pre-fix tie-break survivor was the reply, not the opener.
    write_capture(
        os.path.join(FIXTURES_ROOT, "collapse-actionable-e"),
        base_pr(), base_files(), base_reviews(),
        base_threads(nodes=[
            thread_node("thread-1", comments("discussion_r99", "discussion_r100"), resolved=False)
        ]),
        base_commits(), base_checks(),
    )
    # F (control): identical in every other respect; only the two URL
    # suffixes are swapped, so the same pre-fix tie-break happens to survive
    # on the opener instead.
    write_capture(
        os.path.join(FIXTURES_ROOT, "collapse-actionable-f"),
        base_pr(), base_files(), base_reviews(),
        base_threads(nodes=[
            thread_node("thread-1", comments("discussion_r100", "discussion_r99"), resolved=False)
        ]),
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# colliding check ids (S12): two `CheckRun`s named `build` with no
# `detailsUrl`, one `SUCCESS` and one `FAILURE` — `check_id` hashes the
# absent url as `sha256("")` for both, colliding despite differing
# `conclusion`.
# ---------------------------------------------------------------------
def build_colliding_check_ids():
    fixture_dir = os.path.join(FIXTURES_ROOT, "colliding-check-ids")
    contexts = [
        {"__typename": "CheckRun", "name": "build", "status": "COMPLETED",
         "conclusion": "SUCCESS", "startedAt": "2026-01-01T03:00:00Z",
         "completedAt": "2026-01-01T03:05:00Z"},
        {"__typename": "CheckRun", "name": "build", "status": "COMPLETED",
         "conclusion": "FAILURE", "startedAt": "2026-01-01T03:10:00Z",
         "completedAt": "2026-01-01T03:15:00Z"},
    ]
    write_capture(
        fixture_dir, base_pr(), base_files(), base_reviews(), base_threads(),
        base_commits(), base_checks(contexts=contexts),
    )


# ---------------------------------------------------------------------
# empty pr title (S13): an empty provider `title` string passes the Rust
# owners untouched and produces a record breaching its shipped schema's
# `minLength: 1`.
# ---------------------------------------------------------------------
def build_empty_pr_title():
    fixture_dir = os.path.join(FIXTURES_ROOT, "empty-pr-title")
    pr = base_pr()
    pr["title"] = ""
    write_capture(
        fixture_dir, pr, base_files(), base_reviews(), base_threads(),
        base_commits(), base_checks(),
    )


# ---------------------------------------------------------------------
# duplicate file path (S13): the `files` artifact lists the same path
# twice — a provider impossibility (`changeType` is a fact about one path,
# not two), left unrefused it doubles `can_skim`'s per-file claim.
# ---------------------------------------------------------------------
def build_duplicate_file_path():
    fixture_dir = os.path.join(FIXTURES_ROOT, "duplicate-file-path")
    files = {"files": [
        {"path": "a.rs", "additions": 1, "deletions": 0, "changeType": "ADDED"},
        {"path": "a.rs", "additions": 2, "deletions": 1, "changeType": "MODIFIED"},
    ]}
    write_capture(
        fixture_dir, base_pr(), files, base_reviews(), base_threads(),
        base_commits(), base_checks(),
    )


if __name__ == "__main__":
    build_actor_substitution()
    build_association_not_independence()
    build_actor_set_source_without_ids()
    build_skipped_check()
    build_duplicate_bot_findings()
    build_cross_repository_references()
    build_disappearing_checks()
    build_edited_review_comments()
    build_colliding_finding_ids()
    build_commits_truncation()
    build_single_check_changed()
    build_duplicate_issue_number()
    build_outside_human_commented_review()
    build_truncated_thread()
    build_empty_unresolved_thread()
    build_collapse_actionable_pair()
    build_colliding_check_ids()
    build_empty_pr_title()
    build_duplicate_file_path()
    print("done")

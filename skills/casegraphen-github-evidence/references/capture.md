# Capture recipe and manifest template

The capture is the trust root of everything downstream: `observe` derives the
implementation actor set and every independence classification from these
bytes only. The field selections below are load-bearing, in two different
ways:

- **Refused if missing.** The `pr` and `commits` artifacts must carry GitHub
  node ids (`author{... on Node{id}}`, `user{id}`) — the implementation actor
  set is built from them, and `observe` refuses the whole capture
  (`actor_set_source_missing_id`) rather than falling back to login matching.
  Likewise `totalCount` on every connection: it is how truncation is detected,
  and a `commits` capture whose `totalCount` exceeds its nodes is refused
  (`commits_capture_truncated`) because a truncated actor set cannot anchor
  independence.
- **Silently downgraded if missing.** Actor fields on `reviews`, threads, and
  checks (`__typename`, `id` on authors, `resolvedBy`, `creator`) never refuse:
  an actor without a provider attestation classifies `unattributed`, which
  satisfies no policy and merges with nothing. The capture "works" and the
  classification quietly loses information — no error will ever tell you to
  re-capture. Use the exact selections below; do not trim fields that look
  optional, and do not add fields that assert trust (the manifest has no trust
  vocabulary, and unknown fields are refused).

Both failure modes are only discoverable at `observe` time, after the capture
— and each retry is a live `gh` round-trip against the real repository. That
is why this recipe is written down instead of left to iteration.

## The `gh` commands

For pull request `N` in `OWNER/REPO`, closing issue `M`. One file per manifest
category; `files` reuses the `pr` capture, which already embeds the changed
files.

```sh
mkdir -p capture

gh issue view M -R OWNER/REPO \
  --json number,title,state,stateReason,body,labels,createdAt,closedAt,url,author,closedByPullRequestsReferences \
  > capture/issue-M.json

gh pr view N -R OWNER/REPO \
  --json number,title,body,state,url,author,baseRefName,headRefName,baseRefOid,headRefOid,mergeable,mergeStateStatus,mergedAt,createdAt,files,reviews,statusCheckRollup,latestReviews,comments \
  > capture/pr-N.json

gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){pullRequest(number:N){number reviews(first:100){totalCount nodes{id state body createdAt submittedAt lastEditedAt url authorAssociation commit{oid} author{__typename login ... on Node{id}}}}}}}' \
  > capture/pr-N-reviews.json

gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){pullRequest(number:N){number baseRefOid headRefOid mergeable reviewThreads(first:100){totalCount nodes{id isResolved isOutdated isCollapsed line path resolvedBy{__typename login id} comments(first:20){totalCount nodes{id body createdAt lastEditedAt url authorAssociation author{__typename login ... on Node{id}}}}}}}}}' \
  > capture/pr-N-threads.json

gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){pullRequest(number:N){commits(first:100){totalCount nodes{commit{oid authoredDate committedDate author{user{__typename login id} name email} committer{user{__typename login id} name email}}}}}}}' \
  > capture/pr-N-commits.json

gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){pullRequest(number:N){headRefOid commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(first:50){totalCount nodes{__typename ... on CheckRun{name status conclusion startedAt completedAt detailsUrl checkSuite{app{slug} workflowRun{workflow{name}}}} ... on StatusContext{context state targetUrl description createdAt creator{__typename login ... on Node{id}}}}}}}}}}}}' \
  > capture/pr-N-checks.json
```

If any `totalCount` exceeds the nodes returned, page with `after:` cursors and
concatenate before manifesting — a partial `commits` capture is refused, and a
partial `reviews`/`threads` capture surfaces as a declared truncation loss.

## The manifest template

Seven categories, each exactly once — except `issue`, which appears once per
number in `issue_numbers` and is the only entry that carries `issue_number`.
`command_record` is the argv **array** of the command that produced the file
(a single string is refused). `content_hash` is
`sha256:$(shasum -a 256 <file> | cut -d' ' -f1)` of the exact bytes.
`captured_at` must be `YYYY-MM-DDThh:mm:ssZ`.

```json
{
  "schema": "casegraphen.experimental.github.capture_manifest.v0",
  "repository": "OWNER/REPO",
  "issue_numbers": [M],
  "pr_number": N,
  "captured_at": "2026-08-06T09:00:00Z",
  "capture_tool": "gh",
  "entries": [
    {
      "category": "issue",
      "issue_number": M,
      "artifact_path": "capture/issue-M.json",
      "content_hash": "sha256:<64 hex>",
      "command_record": ["gh", "issue", "view", "M", "-R", "OWNER/REPO", "--json", "number,title,state,stateReason,body,labels,createdAt,closedAt,url,author,closedByPullRequestsReferences"]
    },
    {
      "category": "pr",
      "artifact_path": "capture/pr-N.json",
      "content_hash": "sha256:<64 hex>",
      "command_record": ["gh", "pr", "view", "N", "-R", "OWNER/REPO", "--json", "number,title,body,state,url,author,baseRefName,headRefName,baseRefOid,headRefOid,mergeable,mergeStateStatus,mergedAt,createdAt,files,reviews,statusCheckRollup,latestReviews,comments"]
    },
    {
      "category": "files",
      "artifact_path": "capture/pr-N.json",
      "content_hash": "sha256:<same as pr>",
      "command_record": ["gh", "pr", "view", "N", "-R", "OWNER/REPO", "--json", "number,title,body,state,url,author,baseRefName,headRefName,baseRefOid,headRefOid,mergeable,mergeStateStatus,mergedAt,createdAt,files,reviews,statusCheckRollup,latestReviews,comments"]
    },
    {
      "category": "reviews",
      "artifact_path": "capture/pr-N-reviews.json",
      "content_hash": "sha256:<64 hex>",
      "command_record": ["gh", "api", "graphql", "-f", "query=<the reviews query above>"]
    },
    {
      "category": "review_threads",
      "artifact_path": "capture/pr-N-threads.json",
      "content_hash": "sha256:<64 hex>",
      "command_record": ["gh", "api", "graphql", "-f", "query=<the threads query above>"]
    },
    {
      "category": "commits",
      "artifact_path": "capture/pr-N-commits.json",
      "content_hash": "sha256:<64 hex>",
      "command_record": ["gh", "api", "graphql", "-f", "query=<the commits query above>"]
    },
    {
      "category": "checks",
      "artifact_path": "capture/pr-N-checks.json",
      "content_hash": "sha256:<64 hex>",
      "command_record": ["gh", "api", "graphql", "-f", "query=<the checks query above>"]
    }
  ]
}
```

`artifact_path` is relative to `--capture-dir` and must stay inside it — and
the template's paths carry the `capture/` prefix, so `--capture-dir` is the
directory *containing* `capture/`, not `capture/` itself; pointing at
`capture/` refuses every entry with `artifact_path_escape`. The retained
dogfood pilot (`docs/pilots/issue-102/` in the source tree) is a complete,
real instance of this template if you have the repository checkout.

## Refusals that cost a re-capture

These fire at `observe` time but can only be fixed at capture time — each
retry is a live `gh` round-trip, which is why the recipe above prevents them
instead of letting you iterate into them:

| refusal (exit 1) | fix |
|---|---|
| `content_hash_mismatch` | the file's bytes are not the declared `content_hash` — re-hash, or re-capture; never edit a captured file |
| `commits_capture_truncated` | `totalCount` exceeds captured nodes — re-capture with `after:` pagination |
| `actor_set_source_missing_id` | the `pr`/`commits` capture lacks node ids — re-capture with the field selections above |
| `duplicate_finding_id` / duplicate check id | two records share a provider URL/id, a provider impossibility — the capture is malformed or forged; re-capture, do not dedupe by hand |

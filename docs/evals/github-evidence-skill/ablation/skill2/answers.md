# Answers

## (a) Are the records accepted evidence in a case space?

No. Every record these commands emit carries `accepted: false` (confirmed in
`observe.json` and `project.json`): they are store-free observation proposals,
never accepted facts. A mutation-capable follow-up happens outside this skill,
through `casegraphen-operate`'s gated `evidence attach` against a concrete case
space, where the operation gate is validated before and at append.

## (b) Detecting later that the PR head moved (stale observation)

Re-capture the PR with `gh` into a new directory, author a new manifest, and run
`casegraphen github refresh --manifest new-manifest.json --capture-dir new
--previous-manifest manifest.json --previous-capture-dir . --format json
--output refresh.json` (optionally declaring the basis with
`--previous-observation` set to the bare `result.pr_observation` record
extracted from `observe.json`). A moved head surfaces as the `stale_head`
domain finding (exit 0, or exit 2 with `--strict`); refresh never rebases the
old basis, so the fix is a fresh `github observe` on the new capture. Note
`head_unchanged` is not "nothing changed": `observation_changes` still reports
same-head drift.

## (c) Exact `gh` commands to capture a NEW pull request

For PR `N` in `OWNER/REPO` closing issue `M` (one file per manifest category;
`files` reuses the `pr` capture; if any `totalCount` exceeds returned nodes,
page with `after:` cursors and concatenate before manifesting):

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

The node-id selections (`... on Node{id}`, `user{id}`) and `totalCount` fields
are load-bearing: without them `observe` refuses the capture
(`actor_set_source_missing_id`, `commits_capture_truncated`), and trimmed actor
fields on reviews/threads/checks silently downgrade classification to
`unattributed`.

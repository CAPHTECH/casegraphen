# Answers

## (a) Are these records accepted evidence in a case space?

No. Every record the `github observe|project|refresh` commands emit carries `accepted: false` (confirmed in `observe.json`: `result.accepted = false`, `result.mutation_performed = false`) — they are store-free, proposal-only observation records, never accepted facts. A mutation-capable follow-up would happen through the `casegraphen-operate` skill's gated `evidence attach` against a specific case space, which validates an operation gate before any durable mutation; this GitHub-evidence skill never performs it.

## (b) How would I detect later that the PR head moved and my observation is stale?

Re-capture the PR with the same `gh` recipe into a new directory with a new manifest, then run `casegraphen github refresh --manifest new-manifest.json --capture-dir new --previous-manifest manifest.json --previous-capture-dir . --format json --output refresh.json` (optionally also `--previous-observation` set to the bare `result.pr_observation` record extracted from `observe.json`, which must match the re-normalized previous capture byte-for-byte). A moved head surfaces as the `stale_head` domain finding; refresh never rebases the old basis, so a new `github observe` on the new capture is needed for a new basis — and even `head_unchanged` requires reading `observation_changes` for same-head drift.

## (c) Exact `gh` commands to capture a NEW pull request's state

For pull request `N` in `OWNER/REPO`, closing issue `M` (one file per manifest category; `files` reuses the `pr` capture):

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

If any `totalCount` exceeds the nodes returned, page with `after:` cursors and concatenate before manifesting. The node-id selections (`... on Node{id}`, `user{id}`) and `totalCount` fields are load-bearing: missing ids on `pr`/`commits` refuse the whole capture, and trimmed actor fields elsewhere silently downgrade classification to `unattributed`.

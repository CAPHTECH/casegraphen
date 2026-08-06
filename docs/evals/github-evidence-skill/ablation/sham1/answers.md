# Answers

## (a) Are these records accepted evidence in a case space?

No. `github observe` and `github project` are read-only — the reports carry
`accepted: false` and `mutation_performed: false`, and the usage text states
"observations are never accepted facts; no store access". A mutation-capable
follow-up would go through the store-facing surface against a named case space
(e.g. attaching the retained source records / observation as evidence via a
gated morphism at the current revision), where the material enters as
unreviewed and is promoted only through a canonical review morphism.

## (b) Detecting later that the PR head moved (stale observation)

Capture the PR again, author a new manifest, and run
`casegraphen github refresh --manifest <new-manifest> --capture-dir <new-dir>
--previous-manifest manifest.json --previous-capture-dir capture
--previous-observation observation.json --format json`; it re-normalizes both
captures, compares them, and reports a `stale_head` disposition /
`review_basis_moved` (with per-subject observation changes) when the observed
head no longer equals c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b, and any
projection built on the old basis carries a `stale_head_refresh` finding.

## (c) Exact `gh` commands to capture a NEW pull request (PR N, issue M, repo OWNER/NAME)

```sh
gh pr view N --repo OWNER/NAME --json author,baseRefName,baseRefOid,body,comments,createdAt,files,headRefName,headRefOid,latestReviews,mergeStateStatus,mergeable,mergedAt,number,reviews,state,statusCheckRollup,title,url > pr-N.json

gh issue view M --repo OWNER/NAME --json author,body,closedAt,closedByPullRequestsReferences,createdAt,labels,number,state,stateReason,title,url > issue-M.json

gh api graphql -f query='{repository(owner:"OWNER",name:"NAME"){pullRequest(number:N){number reviews(first:100){totalCount nodes{id state body author{__typename login id} submittedAt lastEditedAt authorAssociation url commit{oid}}}}}}' > pr-N-reviews.json

gh api graphql -f query='{repository(owner:"OWNER",name:"NAME"){pullRequest(number:N){number baseRefOid headRefOid mergeable reviewThreads(first:100){totalCount nodes{id isResolved isOutdated resolvedBy{__typename login id} comments(first:100){totalCount nodes{id body author{__typename login id} url createdAt}}}}}}}' > pr-N-threads.json

gh api graphql -f query='{repository(owner:"OWNER",name:"NAME"){pullRequest(number:N){commits(first:100){totalCount nodes{commit{oid authoredDate committedDate author{user{__typename login id} name email} committer{user{__typename login id} name email}}}}}}}' > pr-N-commits.json

gh api graphql -f query='{repository(owner:"OWNER",name:"NAME"){pullRequest(number:N){headRefOid commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(first:100){totalCount nodes{__typename ... on CheckRun{name status conclusion startedAt completedAt detailsUrl checkSuite{workflowRun{workflow{name}}}} ... on StatusContext{context state description targetUrl createdAt creator{__typename login id}}}}}}}}}}}' > pr-N-checks.json
```

Then record each file's `sha256:` content hash and argv in a
`casegraphen.experimental.github.capture_manifest.v0` manifest (categories:
pr, files, issue, reviews, review_threads, commits, checks — the `files`
category may point at the `gh pr view` capture, which contains `files`), and
run `casegraphen github observe` on it.

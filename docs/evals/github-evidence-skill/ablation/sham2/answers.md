# Answers

## (a) Are these records accepted evidence in a case space?

No. The `github` family is read-only ("observations are never accepted facts; no store access") and both the observation and projection reports carry `accepted: false` and `mutation_performed: false` — they are normalized observations and a derived reviewer projection, not case-space state. A mutation-capable follow-up would go through the gated store surface: attach the observation/projection JSON as evidence with `casegraphen evidence attach --store <dir> --case-space-id <id> --base-revision-id <id> --input <evidence-cell.json> --artifact <path> --gate-profile ...` (born unreviewed), then promote it through the canonical independent-review path (e.g. `packet apply`, which always pauses for an independent review, and `packet resume`/`morphism accept` after that review).

## (b) How would I detect later that the PR head moved and my observation is stale?

Capture the PR again into a fresh directory with a fresh manifest, then run `casegraphen github refresh --manifest <new-manifest> --capture-dir <new-capture-dir> --previous-manifest $DIR/manifest.json --previous-capture-dir $DIR/capture --previous-observation $DIR/observation.json --format json`; it compares the two observations and reports a stale-head finding ("review basis is stale: a refresh observed head X but this review basis is head Y") when the observed `headRefOid` (was `c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b`) no longer matches.

## (c) Exact `gh` commands to capture a NEW pull request's state

For repository `OWNER/REPO`, pull request `N`, closing issue `M` (one fresh directory per capture; each command's argv and the sha256 of each output file go into the manifest):

```sh
# pr + files (one artifact serves both the `pr` and `files` manifest categories)
gh pr view N --repo OWNER/REPO --json author,baseRefName,baseRefOid,body,comments,createdAt,files,headRefName,headRefOid,latestReviews,mergeStateStatus,mergeable,mergedAt,number,reviews,state,statusCheckRollup,title,url > pr-N.json

# reviews
gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){number reviews(first:100){totalCount nodes{id state body createdAt submittedAt lastEditedAt authorAssociation url author{__typename login id} commit{oid}}}}}}' > pr-N-reviews.json

# review threads
gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){number baseRefOid headRefOid mergeable reviewThreads(first:100){totalCount nodes{id isResolved isOutdated isCollapsed line path resolvedBy{__typename login id} comments(first:100){totalCount nodes{id body createdAt lastEditedAt authorAssociation url author{__typename login id}}}}}}}}' > pr-N-threads.json

# commits (implementation actor set needs each commit's GitHub user + node id)
gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){commits(first:250){totalCount nodes{commit{oid authoredDate committedDate author{user{__typename login id} name email} committer{user{__typename login id} name email}}}}}}}' > pr-N-commits.json

# checks at the head commit
gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){headRefOid commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(first:100){totalCount nodes{__typename ... on CheckRun{name status conclusion startedAt completedAt detailsUrl checkSuite{app{slug} workflowRun{workflow{name}}}} ... on StatusContext{context state targetUrl description createdAt creator{__typename login id}}}}}}}}}}}' > pr-N-checks.json

# the closed issue
gh issue view M --repo OWNER/REPO --json author,body,closedAt,closedByPullRequestsReferences,createdAt,labels,number,state,stateReason,title,url > issue-M.json
```

Then author `manifest.json` (schema `casegraphen.experimental.github.capture_manifest.v0`) declaring repository, `pr_number`, `issue_numbers`, `captured_at`, `capture_tool`, and one entry per category (`pr`, `files`, `reviews`, `review_threads`, `commits`, `checks`, `issue`) with `artifact_path`, `content_hash` (`sha256:<hex>` of the exact bytes), and the `command_record` argv array — never editing a captured file afterward.

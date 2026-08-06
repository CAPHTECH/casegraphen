# Answers

## (a) Are these records accepted evidence in a case space, and how would a mutation-capable follow-up happen?

No — both reports say `result.accepted: false` and `result.mutation_performed: false`, and the reviewer projection declares `read_only: true`; they are derived, revision-free observation/projection records, not accepted case-space evidence. A mutation-capable follow-up would run the case-space CLI against an actual case space: attach the observation record (identified by `observation_id` and `normalized_content_hash`) as evidence through a gated, revision-bound evidence-attach operation, where it enters `unreviewed` and is promoted only by a canonical review morphism — the `github` subcommands themselves never mutate anything.

## (b) How would you detect later that the PR head moved and the observation is stale?

Re-capture the PR with `gh` into a new capture dir + manifest and run `casegraphen github refresh --capture-dir <new> --manifest <new-manifest> --previous-capture-dir capture --previous-manifest manifest.json --previous-observation pr-observation.json --format json` — the previous observation must be the bare `pr_observation` record (the tool rejects the full CLI report wrapper). The observation is head-bound: `observation_id` is `github-observation:CAPHTECH/casegraphen#101@c9be9ed6…` and the independence result is pinned to `pr_observation_hash`, so a new capture whose `headRefOid` differs from `c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b` makes refresh report the previous observation stale (and any approval bound to the old head no longer counts).

## (c) Exact `gh` commands to capture a NEW pull request (repo OWNER/REPO, PR N, closing issue M)

```sh
gh pr view N --repo OWNER/REPO \
  --json number,title,state,url,author,body,createdAt,mergedAt,mergeable,mergeStateStatus,baseRefName,baseRefOid,headRefName,headRefOid,files,reviews,latestReviews,comments,statusCheckRollup \
  > capture/pr-N.json

gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='
  query($owner:String!,$name:String!,$number:Int!){ repository(owner:$owner,name:$name){ pullRequest(number:$number){
    number reviews(first:100){ totalCount nodes{ id state body url createdAt submittedAt lastEditedAt authorAssociation
      author{ __typename login ... on User { id } ... on Bot { id } } commit{ oid } } } } } }' \
  > capture/pr-N-reviews.json

gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='
  query($owner:String!,$name:String!,$number:Int!){ repository(owner:$owner,name:$name){ pullRequest(number:$number){
    number baseRefOid headRefOid mergeable
    reviewThreads(first:100){ totalCount nodes{ id path line isResolved isOutdated isCollapsed resolvedBy{ login }
      comments(first:100){ nodes{ id body url createdAt lastEditedAt authorAssociation
        author{ __typename login ... on User { id } ... on Bot { id } } } } } } } } }' \
  > capture/pr-N-threads.json

gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='
  query($owner:String!,$name:String!,$number:Int!){ repository(owner:$owner,name:$name){ pullRequest(number:$number){
    commits(first:250){ totalCount nodes{ commit{ oid authoredDate committedDate
      author{ name email user{ __typename login id } } committer{ name email user{ __typename login id } } } } } } } }' \
  > capture/pr-N-commits.json

gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='
  query($owner:String!,$name:String!,$number:Int!){ repository(owner:$owner,name:$name){ pullRequest(number:$number){
    headRefOid commits(last:1){ nodes{ commit{ oid statusCheckRollup{ state contexts(first:100){ totalCount nodes{
      __typename ... on CheckRun { name status conclusion startedAt completedAt detailsUrl
        checkSuite{ app{ slug } workflowRun{ workflow{ name } } } }
      ... on StatusContext { context state targetUrl createdAt } } } } } } } } } }' \
  > capture/pr-N-checks.json

gh issue view M --repo OWNER/REPO \
  --json number,title,state,stateReason,url,author,body,labels,createdAt,closedAt,closedByPullRequestsReferences \
  > capture/issue-M.json
```

Then write a `casegraphen.experimental.github.capture_manifest.v0` manifest declaring `repository`, `pr_number`, `issue_numbers`, `captured_at`, `capture_tool`, and one entry per artifact (`category` in {pr, files, reviews, review_threads, commits, checks, issue} — exactly one `files` entry, which may reuse `pr-N.json` since `gh pr view --json files` embeds the changed files; issue entries need `issue_number`) with each file's `sha256:<hex>` `content_hash` and the `command_record` argv that produced it.

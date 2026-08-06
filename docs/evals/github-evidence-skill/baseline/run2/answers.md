# Answers

## (a) Are these records accepted evidence in a case space?

No — both reports state `accepted: false`, `mutation_performed: false`, and `read_only: true`: `github observe` and `github project` are read-only derivations that emit unreviewed source records / observation / projection proposals, not accepted case-space evidence. A mutation-capable follow-up would attach the observation report as evidence input to a case space through the gated mutation path (a validated `operation_gate` at an exact revision, with promotion from `unreviewed` happening only via a canonical review morphism), never through these `github` commands themselves.

## (b) Detecting later that the PR head moved (stale observation)

The observation is bound to the observed head (`observation_id: github-observation:CAPHTECH/casegraphen#101@c9be9ed6...`, `head.sha`, and a `normalized_content_hash`), so staleness is detected by taking a fresh `gh` capture and running `casegraphen github refresh --capture-dir <new-capture> --manifest <new-manifest> --format json --previous-observation <prior pr_observation>` — where `--previous-observation` must be the bare `result.pr_observation` record extracted from the observe report (the refresh refusal enumerated its expected fields: `schema`, `observation_id`, `head`, `normalized_content_hash`, ...), and the refresh compares that recorded head/content hash against the new capture's `headRefOid` to report the observation stale when the head moved.

## (c) Exact `gh` commands to capture a NEW pull request's state

For pull request `N` in `OWNER/REPO` (with linked issue `I`), reconstructed from the shapes of the files in `capture/`:

```sh
mkdir -p capture

# 1. Linked issue
gh issue view I --repo OWNER/REPO \
  --json author,body,closedAt,closedByPullRequestsReferences,createdAt,labels,number,state,stateReason,title,url \
  > capture/issue-I.json

# 2. PR core record (includes changed files, latest reviews, status rollup)
gh pr view N --repo OWNER/REPO \
  --json author,baseRefName,baseRefOid,body,comments,createdAt,files,headRefName,headRefOid,latestReviews,mergeStateStatus,mergeable,mergedAt,number,reviews,state,statusCheckRollup,title,url \
  > capture/pr-N.json

# 3. Reviews (GraphQL)
gh api graphql -f owner=OWNER -f name=REPO -F number=N -f query='
query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){ pullRequest(number:$number){
    number
    reviews(first:100){ totalCount nodes{
      id state body authorAssociation createdAt submittedAt lastEditedAt url
      author{ __typename ... on User{ id login } ... on Bot{ id login } }
      commit{ oid }
    } }
  } }
}' > capture/pr-N-reviews.json

# 4. Review threads (GraphQL)
gh api graphql -f owner=OWNER -f name=REPO -F number=N -f query='
query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){ pullRequest(number:$number){
    number baseRefOid headRefOid mergeable
    reviewThreads(first:100){ totalCount nodes{
      id isResolved isOutdated isCollapsed line path
      resolvedBy{ __typename login id }
      comments(first:100){ totalCount nodes{
        id body authorAssociation createdAt lastEditedAt url
        author{ __typename ... on User{ id login } ... on Bot{ id login } }
      } }
    } }
  } }
}' > capture/pr-N-threads.json

# 5. Commits (GraphQL)
gh api graphql -f owner=OWNER -f name=REPO -F number=N -f query='
query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){ pullRequest(number:$number){
    commits(first:250){ totalCount nodes{ commit{
      oid authoredDate committedDate
      author{ user{ __typename login id } name email }
      committer{ user{ __typename login id } name email }
    } } }
  } }
}' > capture/pr-N-commits.json

# 6. Checks at head (GraphQL)
gh api graphql -f owner=OWNER -f name=REPO -F number=N -f query='
query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){ pullRequest(number:$number){
    headRefOid
    commits(last:1){ nodes{ commit{
      oid
      statusCheckRollup{ state contexts(first:100){ totalCount nodes{
        __typename
        ... on CheckRun{ name status conclusion startedAt completedAt detailsUrl
          checkSuite{ app{ slug } workflowRun{ workflow{ name } } } }
        ... on StatusContext{ context state targetUrl createdAt }
      } } }
    } } }
  } }
}' > capture/pr-N-checks.json
```

Then write a capture manifest with `"schema": "casegraphen.experimental.github.capture_manifest.v0"`, `repository`, `issue_numbers`, `pr_number`, `captured_at`, `capture_tool`, and one `entries[]` element per category (`issue`, `pr`, `files`, `reviews`, `review_threads`, `commits`, `checks` — exactly one `files` entry is mandatory; since `gh pr view --json` already includes `files`, the `files` entry can reference `pr-N.json`), each entry carrying `category`, `artifact_path`, `content_hash` as `sha256:<shasum -a 256 of the file>`, `command_record` as the argv array of the `gh` command that produced it, and `issue_number` on the issue entry.

## Note on deliverable 2 (CI exit-code wiring)

`projection.json` was produced with `--require-independent-review`; the flag is this build's wiring for the independent-review policy and it correctly emitted `accepted: false` plus the blocking finding `"require_independent_review is set and no independent human approval is bound to the observed head"` (PR #101 has only COMMENTED reviews from the author and a bot at head `c9be9ed6...`). However, the binary exited 0 in every mode tested (with/without `--output`, with/without the flag), so with this build a CI job keying only on the exit code would NOT fail; the failure is currently visible only inside the report JSON, which conflicts with the requested exit-code-only wiring.

# Answers

## (a) Are these records accepted evidence in a case space?

No. Both reports are read-only observations: the CLI usage states "observations are never accepted facts; no store access", and both reports carry `result.accepted: false`, `result.mutation_performed: false`, and `projection.read_only: true`. A mutation-capable follow-up would attach the observation/projection through the gated store path — `casegraphen evidence attach --store <dir> --case-space-id <id> --base-revision-id <rev> --input <evidence-cell.json> --artifact observation.json` with a validated operation gate (actor, capability, operation scope, audience, source boundary) — producing an unreviewed evidence cell that only becomes accepted through an independent review morphism (`casegraphen review accept ... --reviewer-id <independent-reviewer>`).

## (b) How would I detect later that the PR head moved (stale observation)?

Re-capture the PR with `gh` and run `casegraphen github refresh --capture-dir <new-capture> --manifest <new-manifest> --previous-capture-dir capture --previous-manifest manifest.json --previous-observation observation.json --format json`, which diffs the fresh capture against the prior observation. The staleness is also structurally detectable: the observation id embeds the head SHA (`github-observation:CAPHTECH/casegraphen#101@c9be9ed6...`) and the projection pins `pr_observation_hash`, so a moved head yields a different observation id / normalized content hash than the one the projection is bound to.

## (c) Exact `gh` commands to capture a NEW pull request's state

For PR `N` (closing issue `M`) in `OWNER/REPO`, mirroring the seven manifest categories (`issue`, `pr`, `files`, `reviews`, `review_threads`, `commits`, `checks`):

```sh
mkdir -p capture

# 1. issue
gh issue view M --repo OWNER/REPO \
  --json author,body,closedAt,closedByPullRequestsReferences,createdAt,labels,number,state,stateReason,title,url \
  > capture/issue-M.json

# 2. pr
gh pr view N --repo OWNER/REPO \
  --json author,baseRefName,baseRefOid,body,comments,createdAt,files,headRefName,headRefOid,latestReviews,mergeStateStatus,mergeable,mergedAt,number,reviews,state,statusCheckRollup,title,url \
  > capture/pr-N.json

# 3. files
gh pr view N --repo OWNER/REPO --json files > capture/pr-N-files.json

# 4. reviews (GraphQL)
gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='
query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){ pullRequest(number:$number){
    number
    reviews(first:100){ totalCount nodes{
      id state body createdAt submittedAt lastEditedAt url authorAssociation
      commit{oid}
      author{ __typename login ... on User{id} ... on Bot{id} }
    }}
  }}
}' > capture/pr-N-reviews.json

# 5. review threads (GraphQL)
gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='
query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){ pullRequest(number:$number){
    number baseRefOid headRefOid mergeable
    reviewThreads(first:100){ totalCount nodes{
      id isResolved isOutdated isCollapsed line path resolvedBy{login}
      comments(first:100){ totalCount nodes{
        id body createdAt lastEditedAt url authorAssociation
        author{ __typename login ... on User{id} ... on Bot{id} }
      }}
    }}
  }}
}' > capture/pr-N-threads.json

# 6. commits (GraphQL)
gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='
query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){ pullRequest(number:$number){
    commits(first:250){ totalCount nodes{ commit{
      oid authoredDate committedDate
      author{ user{ __typename login id } name email }
      committer{ user{ __typename login id } name email }
    }}}
  }}
}' > capture/pr-N-commits.json

# 7. checks at head (GraphQL)
gh api graphql -F owner=OWNER -F name=REPO -F number=N -f query='
query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){ pullRequest(number:$number){
    headRefOid
    commits(last:1){ nodes{ commit{
      oid
      statusCheckRollup{ state contexts(first:100){ totalCount nodes{
        __typename
        ... on CheckRun{ name status conclusion startedAt completedAt detailsUrl
          checkSuite{ app{slug} workflowRun{ workflow{name} } } }
        ... on StatusContext{ context state targetUrl createdAt creator{login} }
      }}}
    }}}
  }}
}' > capture/pr-N-checks.json
```

Then write `manifest.json` with `"schema": "casegraphen.experimental.github.capture_manifest.v0"`, `repository`, `issue_numbers`, `pr_number`, `captured_at`, `capture_tool`, and exactly one entry per category, each carrying `category`, `artifact_path`, `content_hash` (`sha256:<hex>` of the file bytes), `command_record` (the exact `gh` argv as a JSON array), and `issue_number` on the issue entry.

## Note on the CI exit-code wiring (task 2)

The projection was produced with the enforcement flag:

```sh
casegraphen github project --capture-dir capture --manifest manifest.json \
  --require-independent-review --format json --output projection.json
```

The report correctly blocks: `result.accepted: false` and blocking finding `independent_review_policy:...@c9be9ed6...` ("require_independent_review is set and no independent human approval is bound to the observed head") — PR #101's only reviews are from the bot `coderabbitai` and the implementation author, so no independent human approval exists at head `c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b`. However, this build exits 0 on the blocked projection (verified directly twice); only refusals (usage/invalid input) exit 1, consistent with the tool's "domain findings are successful results" convention. As observed, a CI job running the command above cannot distinguish blocked from clean via exit code alone without inspecting the JSON report; if exit-code gating is required, the binary would need `--require-independent-review` to escalate the blocking finding to a nonzero exit.

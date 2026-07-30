//! GitHub issue snapshot input contract for the native lift adapter.

use serde::Deserialize;

pub const GITHUB_ISSUE_SNAPSHOT_SCHEMA: &str = "highergraphen.case.github.issue_snapshot.v1";
pub const GITHUB_ISSUE_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GitHubIssueSnapshot {
    pub schema: String,
    pub schema_version: u32,
    pub repository: String,
    pub space_id: String,
    pub query: String,
    pub captured_at: String,
    pub issues: Vec<GitHubIssue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub state: GitHubIssueState,
    #[serde(default, rename = "stateReason")]
    pub state_reason: Option<String>,
    #[serde(default)]
    pub labels: Vec<GitHubIssueLabel>,
    #[serde(default)]
    pub milestone: Option<GitHubIssueMilestone>,
    #[serde(default, rename = "closedByPullRequestsReferences")]
    pub closed_by_pull_requests_references: Vec<GitHubPullRequestReference>,
    #[serde(default, rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(default, rename = "closedAt")]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GitHubIssueState {
    Open,
    Closed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GitHubIssueLabel {
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GitHubIssueMilestone {
    pub number: Option<u64>,
    pub title: String,
    pub description: Option<String>,
    #[serde(rename = "dueOn")]
    pub due_on: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GitHubPullRequestReference {
    pub number: u64,
}

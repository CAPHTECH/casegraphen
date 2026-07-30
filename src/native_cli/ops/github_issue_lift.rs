//! Materializes a bounded GitHub issue snapshot into native cells and relations.

use super::NativeCliError;
use crate::{
    evidence_trust::EvidenceTrustBoundary,
    github_issue_snapshot::{
        GitHubIssue, GitHubIssueSnapshot, GitHubIssueState, GITHUB_ISSUE_SNAPSHOT_SCHEMA,
        GITHUB_ISSUE_SNAPSHOT_SCHEMA_VERSION,
    },
    native_model::{
        CaseCell, CaseCellLifecycle, CaseCellType, CaseRelation, CaseRelationType, RelationStrength,
    },
};
use higher_graphen_core::{Confidence, Id, Provenance, ReviewStatus, SourceKind, SourceRef};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct GitHubIssueMaterialization {
    pub(super) cells: Vec<CaseCell>,
    pub(super) relations: Vec<CaseRelation>,
    pub(super) information_loss: Vec<Value>,
}

pub(super) fn parse_github_issue_snapshot(
    bytes: &[u8],
) -> Result<GitHubIssueSnapshot, NativeCliError> {
    let snapshot: GitHubIssueSnapshot = serde_json::from_slice(bytes)?;
    if snapshot.schema != GITHUB_ISSUE_SNAPSHOT_SCHEMA {
        return Err(NativeCliError::invalid(format!(
            "unsupported GitHub issue snapshot schema {:?}; expected \
             {GITHUB_ISSUE_SNAPSHOT_SCHEMA:?}",
            snapshot.schema
        )));
    }
    if snapshot.schema_version != GITHUB_ISSUE_SNAPSHOT_SCHEMA_VERSION {
        return Err(NativeCliError::invalid(format!(
            "unsupported GitHub issue snapshot schema version {}; expected \
             {GITHUB_ISSUE_SNAPSHOT_SCHEMA_VERSION}",
            snapshot.schema_version
        )));
    }
    for (field, value) in [
        ("repository", snapshot.repository.as_str()),
        ("space_id", snapshot.space_id.as_str()),
        ("query", snapshot.query.as_str()),
        ("captured_at", snapshot.captured_at.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(NativeCliError::invalid(format!(
                "GitHub issue snapshot {field} must not be empty"
            )));
        }
    }
    Id::new(snapshot.space_id.clone())?;
    let mut issue_numbers = BTreeSet::new();
    for issue in &snapshot.issues {
        if issue.number == 0 {
            return Err(NativeCliError::invalid(
                "GitHub issue number must be greater than zero",
            ));
        }
        if !issue_numbers.insert(issue.number) {
            return Err(NativeCliError::invalid(format!(
                "GitHub issue snapshot contains duplicate issue number {}",
                issue.number
            )));
        }
        if issue.title.trim().is_empty() {
            return Err(NativeCliError::invalid(format!(
                "GitHub issue #{} title must not be empty",
                issue.number
            )));
        }
        for label in &issue.labels {
            if label.name.trim().is_empty() {
                return Err(NativeCliError::invalid(format!(
                    "GitHub issue #{} has an empty label name",
                    issue.number
                )));
            }
        }
        if let Some(milestone) = &issue.milestone {
            if milestone.title.trim().is_empty() {
                return Err(NativeCliError::invalid(format!(
                    "GitHub issue #{} has an empty milestone title",
                    issue.number
                )));
            }
        }
        if issue
            .closed_by_pull_requests_references
            .iter()
            .any(|reference| reference.number == 0)
        {
            return Err(NativeCliError::invalid(format!(
                "GitHub issue #{} has a pull-request reference numbered zero",
                issue.number
            )));
        }
    }
    Ok(snapshot)
}

pub(super) fn materialize_github_issue_snapshot(
    snapshot: &GitHubIssueSnapshot,
) -> Result<GitHubIssueMaterialization, NativeCliError> {
    let source_id = github_source_id(&snapshot.repository)?;
    let space_id = Id::new(snapshot.space_id.clone())?;
    let issue_numbers = snapshot
        .issues
        .iter()
        .map(|issue| issue.number)
        .collect::<BTreeSet<_>>();
    let mut cells = Vec::new();
    let mut relations = Vec::new();
    let mut milestone_ids = BTreeMap::<String, Id>::new();
    let mut milestone_titles_by_id = BTreeMap::<Id, String>::new();
    let mut pull_request_numbers = BTreeSet::new();
    let mut relation_keys = BTreeSet::new();
    let mut skipped_task_targets = BTreeSet::new();

    for issue in &snapshot.issues {
        cells.push(issue_cell(snapshot, issue, &space_id, &source_id)?);

        if let Some(milestone) = &issue.milestone {
            let goal_id = if let Some(goal_id) = milestone_ids.get(&milestone.title) {
                goal_id.clone()
            } else {
                let goal_id = Id::new(format!(
                    "goal:milestone-{}",
                    milestone_slug(&milestone.title)?
                ))?;
                if let Some(existing_title) = milestone_titles_by_id.get(&goal_id) {
                    if existing_title != &milestone.title {
                        return Err(NativeCliError::invalid(format!(
                            "GitHub milestone titles {existing_title:?} and {:?} produce the \
                             same goal id {goal_id}",
                            milestone.title
                        )));
                    }
                }
                cells.push(milestone_cell(
                    snapshot,
                    &space_id,
                    &source_id,
                    &goal_id,
                    &milestone.title,
                )?);
                milestone_ids.insert(milestone.title.clone(), goal_id.clone());
                milestone_titles_by_id.insert(goal_id.clone(), milestone.title.clone());
                goal_id
            };
            let issue_id = issue_id(issue.number)?;
            relations.push(github_relation(
                snapshot,
                Id::new(format!(
                    "relation:github:issue-{}:covers:{}",
                    issue.number,
                    milestone_slug(&milestone.title)?
                ))?,
                CaseRelationType::Covers,
                RelationStrength::Diagnostic,
                issue_id,
                goal_id,
                Vec::new(),
                &source_id,
                issue.number.to_string(),
                "github_milestone",
            )?);
        }

        for reference in &issue.closed_by_pull_requests_references {
            let evidence_id = Id::new(format!("evidence:github-pr-{}", reference.number))?;
            if pull_request_numbers.insert(reference.number) {
                cells.push(pull_request_evidence_cell(
                    snapshot,
                    &space_id,
                    &source_id,
                    reference.number,
                    &evidence_id,
                )?);
            }
            let key = (reference.number, issue.number);
            if relation_keys.insert(("verifies", key.0, key.1)) {
                relations.push(github_relation(
                    snapshot,
                    Id::new(format!(
                        "relation:github:pr-{}:verifies:issue-{}",
                        reference.number, issue.number
                    ))?,
                    CaseRelationType::Verifies,
                    RelationStrength::Diagnostic,
                    evidence_id.clone(),
                    issue_id(issue.number)?,
                    vec![evidence_id],
                    &source_id,
                    reference.number.to_string(),
                    "github_closed_by_pull_requests_references",
                )?);
            }
        }

        if let Some(body) = &issue.body {
            for target_number in body.lines().filter_map(task_list_issue_number) {
                if !issue_numbers.contains(&target_number) {
                    skipped_task_targets.insert(target_number);
                    continue;
                }
                if relation_keys.insert(("depends_on", issue.number, target_number)) {
                    relations.push(github_relation(
                        snapshot,
                        Id::new(format!(
                            "relation:github:issue-{}:depends-on:issue-{target_number}",
                            issue.number
                        ))?,
                        CaseRelationType::DependsOn,
                        RelationStrength::Soft,
                        issue_id(issue.number)?,
                        issue_id(target_number)?,
                        Vec::new(),
                        &source_id,
                        issue.number.to_string(),
                        "github_task_list_reference",
                    )?);
                }
            }
        }
    }

    // ADR 0003 decision 4 applies to every bounded external lift: the source
    // has no authorization concept, so inventing grants would manufacture a
    // trust root from caller input.
    if let Some(cell) = cells
        .iter()
        .find(|cell| cell.cell_type == CaseCellType::Custom("capability".to_owned()))
    {
        return Err(NativeCliError::invalid(format!(
            "GitHub issue lift cannot materialize capability cell {}: capability cells are \
             administered only in a native genesis inside the declared source boundary",
            cell.id
        )));
    }

    let mut information_loss = vec![
        loss("Issue comments were not fetched or materialized."),
        loss("Issue reactions were not fetched or materialized."),
        loss("Issue assignees were not fetched or materialized."),
        loss("GitHub Projects membership and fields were not fetched or materialized."),
        loss(
            "Issue bodies were not materialized except for line-anchored GitHub task-list \
             references to issue numbers.",
        ),
        loss("Issue timelines and event history were not fetched or materialized."),
        loss(
            "GitHub label names were preserved as metadata only; label semantics were not \
             inferred.",
        ),
        loss(
            "Milestone and pull-request-reference relations defaulted to diagnostic strength; \
             task-list dependency relations defaulted to soft strength.",
        ),
    ];
    if !skipped_task_targets.is_empty() {
        information_loss.push(json!({
            "description": "Task-list references whose target issue was absent from the snapshot \
                            were not materialized as relations.",
            "skipped_issue_numbers": skipped_task_targets,
        }));
    }

    Ok(GitHubIssueMaterialization {
        cells,
        relations,
        information_loss,
    })
}

pub(super) fn github_source_id(repository: &str) -> Result<Id, NativeCliError> {
    Ok(Id::new(format!("source:github:{repository}"))?)
}

pub(super) fn github_source_boundary(
    snapshot: &GitHubIssueSnapshot,
    source_boundary_id: Id,
    source_id: &Id,
    information_loss: Vec<Value>,
) -> Value {
    json!({
        "id": source_boundary_id,
        "included_sources": [{
            "source_id": source_id,
            "repository": snapshot.repository,
            "query": snapshot.query,
            "captured_at": snapshot.captured_at,
        }],
        "excluded_sources": [
            "closed-issue bodies beyond the query limit",
            "pull requests themselves",
            "GitHub Discussions",
        ],
        "adapters": ["github-issues"],
        "accepted_fact_policy": "Issue fields returned by the recorded gh query are accepted as \
                                 bounded API snapshot facts; generated evidence remains \
                                 unreviewed.",
        "inference_policy": "Milestone and pull-request associations are diagnostic; task-list \
                             dependencies are soft and references outside the snapshot are \
                             declared as information loss.",
        "information_loss": information_loss,
    })
}

fn issue_cell(
    snapshot: &GitHubIssueSnapshot,
    issue: &GitHubIssue,
    space_id: &Id,
    source_id: &Id,
) -> Result<CaseCell, NativeCliError> {
    let label_names = issue
        .labels
        .iter()
        .map(|label| label.name.clone())
        .collect::<Vec<_>>();
    let mut metadata = Map::new();
    metadata.insert("github_state".to_owned(), json!(state_name(issue.state)));
    metadata.insert("github_state_reason".to_owned(), json!(issue.state_reason));
    metadata.insert("github_labels".to_owned(), json!(label_names));
    metadata.insert("github_created_at".to_owned(), json!(issue.created_at));
    metadata.insert("github_closed_at".to_owned(), json!(issue.closed_at));
    Ok(CaseCell {
        id: issue_id(issue.number)?,
        cell_type: CaseCellType::Work,
        space_id: space_id.clone(),
        title: issue.title.clone(),
        summary: None,
        lifecycle: issue_lifecycle(issue),
        source_ids: vec![source_id.clone()],
        structure_ids: Vec::new(),
        provenance: github_provenance(
            snapshot,
            format!("issues/{}", issue.number),
            issue.number.to_string(),
            ReviewStatus::Reviewed,
        )?,
        metadata,
    })
}

fn milestone_cell(
    snapshot: &GitHubIssueSnapshot,
    space_id: &Id,
    source_id: &Id,
    goal_id: &Id,
    title: &str,
) -> Result<CaseCell, NativeCliError> {
    let mut metadata = Map::new();
    metadata.insert("github_milestone_title".to_owned(), json!(title));
    Ok(CaseCell {
        id: goal_id.clone(),
        cell_type: CaseCellType::Goal,
        space_id: space_id.clone(),
        title: title.to_owned(),
        summary: None,
        lifecycle: CaseCellLifecycle::Active,
        source_ids: vec![source_id.clone()],
        structure_ids: Vec::new(),
        provenance: github_provenance(
            snapshot,
            String::new(),
            format!("milestone:{title}"),
            ReviewStatus::Reviewed,
        )?,
        metadata,
    })
}

fn pull_request_evidence_cell(
    snapshot: &GitHubIssueSnapshot,
    space_id: &Id,
    source_id: &Id,
    number: u64,
    evidence_id: &Id,
) -> Result<CaseCell, NativeCliError> {
    let mut metadata = Map::new();
    metadata.insert(
        "evidence_boundary".to_owned(),
        json!(EvidenceTrustBoundary::Inferred.metadata_value()),
    );
    metadata.insert("github_pull_request_number".to_owned(), json!(number));
    // "A PR closed it" is not evidence that the work's requirement was met,
    // and per ADR 0003 every lifted evidence cell enters unreviewed.
    Ok(CaseCell {
        id: evidence_id.clone(),
        cell_type: CaseCellType::Evidence,
        space_id: space_id.clone(),
        title: format!("GitHub PR #{number}"),
        summary: None,
        lifecycle: CaseCellLifecycle::Active,
        source_ids: vec![source_id.clone()],
        structure_ids: Vec::new(),
        provenance: github_provenance(
            snapshot,
            format!("pull/{number}"),
            format!("pull:{number}"),
            ReviewStatus::Unreviewed,
        )?,
        metadata,
    })
}

#[allow(clippy::too_many_arguments)]
fn github_relation(
    snapshot: &GitHubIssueSnapshot,
    id: Id,
    relation_type: CaseRelationType,
    relation_strength: RelationStrength,
    from_id: Id,
    to_id: Id,
    evidence_ids: Vec<Id>,
    source_id: &Id,
    source_local_id: String,
    lifted_from: &str,
) -> Result<CaseRelation, NativeCliError> {
    let mut metadata = Map::new();
    metadata.insert("lifted_from".to_owned(), json!(lifted_from));
    Ok(CaseRelation {
        id,
        relation_type,
        relation_strength,
        from_id,
        to_id,
        evidence_ids,
        source_ids: vec![source_id.clone()],
        provenance: github_provenance(
            snapshot,
            String::new(),
            source_local_id,
            ReviewStatus::Reviewed,
        )?,
        metadata,
    })
}

fn github_provenance(
    snapshot: &GitHubIssueSnapshot,
    repository_path: String,
    source_local_id: String,
    review_status: ReviewStatus,
) -> Result<Provenance, NativeCliError> {
    let mut source = SourceRef::new(SourceKind::Api);
    source.uri = Some(if repository_path.is_empty() {
        format!("https://github.com/{}", snapshot.repository)
    } else {
        format!(
            "https://github.com/{}/{}",
            snapshot.repository, repository_path
        )
    });
    source.title = Some(format!("GitHub repository {}", snapshot.repository));
    source.captured_at = Some(snapshot.captured_at.clone());
    source.source_local_id = Some(source_local_id);
    let confidence =
        Confidence::new(0.9).map_err(|error| NativeCliError::invalid(error.to_string()))?;
    let mut provenance = Provenance::new(source, confidence).with_review_status(review_status);
    provenance.extraction_method = Some("github_issue_snapshot_lift".to_owned());
    Ok(provenance)
}

fn issue_id(number: u64) -> Result<Id, NativeCliError> {
    Ok(Id::new(format!("work:issue-{number}"))?)
}

fn issue_lifecycle(issue: &GitHubIssue) -> CaseCellLifecycle {
    match issue.state {
        GitHubIssueState::Open => CaseCellLifecycle::Active,
        GitHubIssueState::Closed if issue.state_reason.as_deref() == Some("NOT_PLANNED") => {
            CaseCellLifecycle::Retired
        }
        GitHubIssueState::Closed => CaseCellLifecycle::Resolved,
    }
}

fn state_name(state: GitHubIssueState) -> &'static str {
    match state {
        GitHubIssueState::Open => "OPEN",
        GitHubIssueState::Closed => "CLOSED",
    }
}

fn milestone_slug(title: &str) -> Result<String, NativeCliError> {
    let mut slug = String::new();
    let mut pending_separator = false;
    for character in title.trim().chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
            pending_separator = false;
        } else if !slug.is_empty() {
            pending_separator = true;
        }
    }
    if slug.is_empty() {
        return Err(NativeCliError::invalid(format!(
            "GitHub milestone title {title:?} has no ASCII letters or digits for its goal id"
        )));
    }
    Ok(slug)
}

fn task_list_issue_number(line: &str) -> Option<u64> {
    let line = line.trim_start();
    let after_bullet = line
        .strip_prefix('-')
        .or_else(|| line.strip_prefix('*'))?
        .trim_start();
    let bytes = after_bullet.as_bytes();
    if bytes.len() < 3
        || bytes[0] != b'['
        || !matches!(bytes[1], b' ' | b'x' | b'X')
        || bytes[2] != b']'
    {
        return None;
    }
    let after_checkbox = after_bullet[3..].trim_start();
    let digits = after_checkbox.strip_prefix('#')?;
    let digit_count = digits
        .chars()
        .take_while(char::is_ascii_digit)
        .map(char::len_utf8)
        .sum::<usize>();
    if digit_count == 0 {
        return None;
    }
    digits[..digit_count].parse().ok()
}

fn loss(description: &str) -> Value {
    json!({ "description": description })
}

#[cfg(test)]
mod tests {
    use super::task_list_issue_number;

    #[test]
    fn task_list_parser_accepts_only_line_anchored_github_issue_references() {
        assert_eq!(task_list_issue_number("  - [ ] #12 task"), Some(12));
        assert_eq!(task_list_issue_number("\t* [x]#34"), Some(34));
        assert_eq!(task_list_issue_number("* [X] #56"), Some(56));
        assert_eq!(task_list_issue_number("text - [ ] #12"), None);
        assert_eq!(task_list_issue_number("- [] #12"), None);
        assert_eq!(task_list_issue_number("- [y] #12"), None);
        assert_eq!(task_list_issue_number("- [ ] owner/repo#12"), None);
    }
}

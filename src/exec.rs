use crate::native_model::{
    CaseCellLifecycle, CaseCellType, CaseMorphism, CaseMorphismType, CaseSpace, MorphismPayload,
};
use higher_graphen_core::{Id, Provenance, ReviewStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

pub mod binding;
pub mod records;
pub mod worker;

pub const EXECUTION_PLAN_SCHEMA: &str = "highergraphen.case.workflow.execution_plan.v1";
pub const EXECUTION_PLAN_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPlan {
    pub schema: String,
    pub schema_version: u32,
    pub plan_id: Id,
    pub case_space_id: Id,
    pub base_revision_id: Id,
    pub steps: Vec<ExecutionStep>,
    pub provenance: Provenance,
    pub review_status: ReviewStatus,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionStep {
    pub step_id: Id,
    pub work_cell_id: Id,
    pub worker_binding_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_projection_id: Option<Id>,
    pub success_evidence_requirement_ids: Vec<Id>,
    pub allowed_transition_classes: Vec<AllowedTransitionClass>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedTransitionClass {
    pub morphism_type: CaseMorphismType,
    pub target_cell_types: Vec<CaseCellType>,
    pub to_lifecycles: Vec<CaseCellLifecycle>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionPlanValidationError {
    message: String,
}

impl ExecutionPlanValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ExecutionPlanValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ExecutionPlanValidationError {}

pub fn validate_execution_plan(plan: &ExecutionPlan) -> Result<(), ExecutionPlanValidationError> {
    if plan.schema != EXECUTION_PLAN_SCHEMA {
        return Err(ExecutionPlanValidationError::new(format!(
            "unsupported execution plan schema {:?}; expected {EXECUTION_PLAN_SCHEMA:?}",
            plan.schema
        )));
    }
    if plan.schema_version != EXECUTION_PLAN_SCHEMA_VERSION {
        return Err(ExecutionPlanValidationError::new(format!(
            "unsupported execution plan schema version {}; expected {EXECUTION_PLAN_SCHEMA_VERSION}",
            plan.schema_version
        )));
    }
    if plan.steps.is_empty() {
        return Err(ExecutionPlanValidationError::new(
            "execution plan steps must not be empty",
        ));
    }
    if plan.review_status == ReviewStatus::Candidate
        || plan.provenance.review_status == ReviewStatus::Candidate
    {
        return Err(ExecutionPlanValidationError::new(
            "execution plan review statuses must use the execution plan wire values",
        ));
    }
    plan.provenance
        .validate()
        .map_err(|error| ExecutionPlanValidationError::new(error.to_string()))?;

    validate_id("plan_id", &plan.plan_id)?;
    validate_id("case_space_id", &plan.case_space_id)?;
    validate_id("base_revision_id", &plan.base_revision_id)?;
    for (index, step) in plan.steps.iter().enumerate() {
        validate_id(&format!("steps[{index}].step_id"), &step.step_id)?;
        validate_id(&format!("steps[{index}].work_cell_id"), &step.work_cell_id)?;
        validate_id(
            &format!("steps[{index}].worker_binding_id"),
            &step.worker_binding_id,
        )?;
        if let Some(input_projection_id) = &step.input_projection_id {
            validate_id(
                &format!("steps[{index}].input_projection_id"),
                input_projection_id,
            )?;
        }
        for (evidence_index, evidence_id) in
            step.success_evidence_requirement_ids.iter().enumerate()
        {
            validate_id(
                &format!("steps[{index}].success_evidence_requirement_ids[{evidence_index}]"),
                evidence_id,
            )?;
        }
        if step.allowed_transition_classes.is_empty() {
            return Err(ExecutionPlanValidationError::new(format!(
                "steps[{index}].allowed_transition_classes must not be empty"
            )));
        }
    }
    Ok(())
}

fn validate_id(label: &str, id: &Id) -> Result<(), ExecutionPlanValidationError> {
    if Id::is_valid_value(id.as_str()) {
        Ok(())
    } else {
        Err(ExecutionPlanValidationError::new(format!(
            "{label} is not a well-formed id"
        )))
    }
}

pub fn transition_permitted(
    allowed_transition_class: &AllowedTransitionClass,
    morphism: &CaseMorphism,
    case_space: &CaseSpace,
) -> bool {
    if morphism.morphism_type != allowed_transition_class.morphism_type {
        return false;
    }
    let payload = match morphism.metadata.get("payload") {
        Some(value) => match serde_json::from_value::<MorphismPayload>(value.clone()) {
            Ok(payload) => payload,
            Err(_) => return false,
        },
        None => MorphismPayload::default(),
    };

    if !allowed_transition_class.target_cell_types.is_empty()
        && payload
            .added_cells
            .iter()
            .chain(&payload.updated_cells)
            .any(|cell| {
                !allowed_transition_class
                    .target_cell_types
                    .contains(&cell.cell_type)
            })
    {
        return false;
    }

    if allowed_transition_class.to_lifecycles.is_empty() {
        return true;
    }
    let payload_lifecycles_permitted = payload
        .added_cells
        .iter()
        .chain(&payload.updated_cells)
        .all(|cell| {
            allowed_transition_class
                .to_lifecycles
                .contains(&cell.lifecycle)
        });
    let retired_cells_permitted = morphism.retired_ids.iter().all(|retired_id| {
        case_space
            .case_cells
            .iter()
            .find(|cell| cell.id == *retired_id)
            .map_or(true, |_| {
                allowed_transition_class
                    .to_lifecycles
                    .contains(&CaseCellLifecycle::Retired)
            })
    });
    payload_lifecycles_permitted && retired_cells_permitted
}

pub fn execution_plan_content_hash(plan: &ExecutionPlan) -> Result<String, serde_json::Error> {
    let mut normalized = plan.clone();
    normalized.review_status = ReviewStatus::Unreviewed;
    let canonical = serde_json::to_string(&serde_json::to_value(normalized)?)?;
    Ok(crate::native_hash::sha256_hex(canonical.as_bytes()))
}

pub fn accepted_plan_content_hash_matches(
    plan: &ExecutionPlan,
    recorded_content_hash: &str,
) -> Result<bool, serde_json::Error> {
    Ok(execution_plan_content_hash(plan)? == recorded_content_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const EXECUTION_PLAN_EXAMPLE: &str =
        include_str!("../schemas/casegraphen/execution.plan.example.json");
    const NATIVE_CASE_SPACE_EXAMPLE: &str =
        include_str!("../schemas/casegraphen/native.case.space.example.json");

    #[test]
    fn execution_plan_example_validates_and_round_trips() {
        let plan: ExecutionPlan =
            serde_json::from_str(EXECUTION_PLAN_EXAMPLE).expect("execution plan example");

        validate_execution_plan(&plan).expect("valid execution plan");
        let round_trip: ExecutionPlan =
            serde_json::from_value(serde_json::to_value(&plan).expect("serialize plan"))
                .expect("deserialize plan");

        assert_eq!(round_trip, plan);
        assert_eq!(plan.schema, EXECUTION_PLAN_SCHEMA);
        assert_eq!(plan.schema_version, EXECUTION_PLAN_SCHEMA_VERSION);
    }

    #[test]
    fn transition_permitted_checks_type_cell_type_and_lifecycle() {
        let case_space: CaseSpace =
            serde_json::from_str(NATIVE_CASE_SPACE_EXAMPLE).expect("native case space example");
        let work_cell = case_space
            .case_cells
            .iter()
            .find(|cell| cell.id.as_str() == "work:review-native-contract")
            .expect("work cell")
            .clone();
        let mut updated_cell = work_cell.clone();
        updated_cell.lifecycle = CaseCellLifecycle::Resolved;
        let mut morphism: CaseMorphism = serde_json::from_value(json!({
            "morphism_id": "morphism:transition-matrix",
            "morphism_type": "update",
            "source_revision_id": "revision:native-contract-v1",
            "target_revision_id": "revision:transition-matrix",
            "added_ids": [],
            "updated_ids": ["work:review-native-contract"],
            "retired_ids": [],
            "preserved_ids": [],
            "violated_invariant_ids": [],
            "review_status": "unreviewed",
            "evidence_ids": [],
            "source_ids": [],
            "metadata": {
                "payload": {
                    "added_cells": [],
                    "added_relations": [],
                    "updated_cells": [updated_cell],
                    "updated_relations": []
                }
            }
        }))
        .expect("morphism");
        let permitted = AllowedTransitionClass {
            morphism_type: CaseMorphismType::Update,
            target_cell_types: vec![CaseCellType::Work],
            to_lifecycles: vec![CaseCellLifecycle::Resolved],
        };

        assert!(transition_permitted(&permitted, &morphism, &case_space));

        let mut wrong_type = permitted.clone();
        wrong_type.morphism_type = CaseMorphismType::Create;
        assert!(!transition_permitted(&wrong_type, &morphism, &case_space));

        let mut wrong_cell_type = permitted.clone();
        wrong_cell_type.target_cell_types = vec![CaseCellType::Goal];
        assert!(!transition_permitted(
            &wrong_cell_type,
            &morphism,
            &case_space
        ));

        let mut wrong_lifecycle = permitted.clone();
        wrong_lifecycle.to_lifecycles = vec![CaseCellLifecycle::Accepted];
        assert!(!transition_permitted(
            &wrong_lifecycle,
            &morphism,
            &case_space
        ));

        morphism.updated_ids.clear();
        morphism.retired_ids = vec![work_cell.id];
        morphism
            .metadata
            .insert("payload".to_owned(), json!(MorphismPayload::default()));
        let retire_class = AllowedTransitionClass {
            morphism_type: CaseMorphismType::Update,
            target_cell_types: Vec::new(),
            to_lifecycles: vec![CaseCellLifecycle::Retired],
        };
        assert!(transition_permitted(&retire_class, &morphism, &case_space));
        assert!(!transition_permitted(
            &wrong_lifecycle,
            &morphism,
            &case_space
        ));
    }

    #[test]
    fn accepted_plan_hash_normalizes_review_status_to_unreviewed() {
        let mut plan: ExecutionPlan =
            serde_json::from_str(EXECUTION_PLAN_EXAMPLE).expect("execution plan example");
        let proposed_hash = execution_plan_content_hash(&plan).expect("proposed plan hash");
        plan.review_status = ReviewStatus::Accepted;

        assert!(accepted_plan_content_hash_matches(&plan, &proposed_hash)
            .expect("accepted plan hash verification"));
        plan.metadata
            .insert("tampered".to_owned(), Value::Bool(true));
        assert!(!accepted_plan_content_hash_matches(&plan, &proposed_hash)
            .expect("tampered plan hash verification"));
    }
}

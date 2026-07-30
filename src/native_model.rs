use crate::evidence_trust::{EvidenceTrustBoundary, EvidenceTrustInput};
use higher_graphen_core::{Id, Provenance, ReviewStatus, Severity};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
};

pub const NATIVE_CASE_SPACE_SCHEMA: &str = "highergraphen.case.space.v1";
pub const NATIVE_CASE_SPACE_SCHEMA_VERSION: u32 = 1;
pub const NATIVE_MORPHISM_LOG_ENTRY_SCHEMA: &str = "highergraphen.case.morphism_log_entry.v1";

const CUSTOM_PREFIX: &str = "custom:";

pub type MorphismLog = Vec<MorphismLogEntry>;

macro_rules! impl_custom_enum {
    ($name:ident, { $($value:literal => $variant:ident),+ $(,)? }) => {
        impl $name {
            pub fn serialized_value(&self) -> String {
                match self {
                    $(Self::$variant => $value.to_owned(),)+
                    Self::Custom(extension) => format!("{CUSTOM_PREFIX}{extension}"),
                }
            }
        }

        impl FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    custom if custom.starts_with(CUSTOM_PREFIX) => {
                        let extension = &custom[CUSTOM_PREFIX.len()..];
                        if extension.trim().is_empty() {
                            Err(format!("{value:?} has an empty custom extension"))
                        } else {
                            Ok(Self::Custom(extension.to_owned()))
                        }
                    }
                    unknown => Err(format!("unsupported {} value {unknown:?}", stringify!($name))),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.serialized_value())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::from_str(&value).map_err(de::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.serialized_value())
            }
        }
    };
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseSpace {
    pub schema: String,
    pub schema_version: u32,
    pub case_space_id: Id,
    pub space_id: Id,
    pub case_cells: Vec<CaseCell>,
    pub case_relations: Vec<CaseRelation>,
    pub morphism_log: MorphismLog,
    pub projections: Vec<Projection>,
    pub revision: Revision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_policy_id: Option<Id>,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseCell {
    pub id: Id,
    pub cell_type: CaseCellType,
    pub space_id: Id,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub lifecycle: CaseCellLifecycle,
    pub source_ids: Vec<Id>,
    pub structure_ids: Vec<Id>,
    pub provenance: Provenance,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CaseCellType {
    Case,
    Scenario,
    Goal,
    Work,
    Decision,
    Event,
    Evidence,
    Proof,
    Review,
    Obstruction,
    Completion,
    Projection,
    Revision,
    Morphism,
    ExternalRef,
    Custom(String),
}

impl_custom_enum!(
    CaseCellType,
    {
        "case" => Case,
        "scenario" => Scenario,
        "goal" => Goal,
        "work" => Work,
        "decision" => Decision,
        "event" => Event,
        "evidence" => Evidence,
        "proof" => Proof,
        "review" => Review,
        "obstruction" => Obstruction,
        "completion" => Completion,
        "projection" => Projection,
        "revision" => Revision,
        "morphism" => Morphism,
        "external_ref" => ExternalRef
    }
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseCellLifecycle {
    Proposed,
    Active,
    Waiting,
    Resolved,
    Retired,
    Accepted,
    Rejected,
    Superseded,
}

impl CaseCellLifecycle {
    pub fn can_transition_to(self, target: Self) -> bool {
        self == target
            || matches!(
                (self, target),
                (
                    Self::Proposed,
                    Self::Active | Self::Rejected | Self::Retired
                ) | (
                    Self::Active,
                    Self::Waiting | Self::Resolved | Self::Retired | Self::Superseded
                ) | (Self::Waiting, Self::Active | Self::Retired)
                    | (
                        Self::Resolved,
                        Self::Accepted | Self::Active | Self::Retired
                    )
                    | (Self::Accepted, Self::Superseded | Self::Retired)
                    | (Self::Rejected | Self::Superseded, Self::Retired)
            )
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseRelation {
    pub id: Id,
    pub relation_type: CaseRelationType,
    pub relation_strength: RelationStrength,
    pub from_id: Id,
    pub to_id: Id,
    pub evidence_ids: Vec<Id>,
    pub source_ids: Vec<Id>,
    pub provenance: Provenance,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CaseRelationType {
    DependsOn,
    WaitsFor,
    RequiresEvidence,
    RequiresProof,
    SatisfiesEvidenceRequirement,
    Verifies,
    Covers,
    Exercises,
    Blocks,
    Unblocks,
    Contradicts,
    Invalidates,
    Completes,
    DerivesFrom,
    Refines,
    ProjectsTo,
    TransitionsTo,
    CorrespondsTo,
    Accepts,
    Rejects,
    Supersedes,
    Custom(String),
}

impl_custom_enum!(
    CaseRelationType,
    {
        "depends_on" => DependsOn,
        "waits_for" => WaitsFor,
        "requires_evidence" => RequiresEvidence,
        "requires_proof" => RequiresProof,
        "satisfies_evidence_requirement" => SatisfiesEvidenceRequirement,
        "verifies" => Verifies,
        "covers" => Covers,
        "exercises" => Exercises,
        "blocks" => Blocks,
        "unblocks" => Unblocks,
        "contradicts" => Contradicts,
        "invalidates" => Invalidates,
        "completes" => Completes,
        "derives_from" => DerivesFrom,
        "refines" => Refines,
        "projects_to" => ProjectsTo,
        "transitions_to" => TransitionsTo,
        "corresponds_to" => CorrespondsTo,
        "accepts" => Accepts,
        "rejects" => Rejects,
        "supersedes" => Supersedes
    }
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationStrength {
    Hard,
    Soft,
    Diagnostic,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseMorphism {
    pub morphism_id: Id,
    pub morphism_type: CaseMorphismType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_revision_id: Option<Id>,
    pub target_revision_id: Id,
    pub added_ids: Vec<Id>,
    pub updated_ids: Vec<Id>,
    pub retired_ids: Vec<Id>,
    pub preserved_ids: Vec<Id>,
    pub violated_invariant_ids: Vec<Id>,
    pub review_status: ReviewStatus,
    pub evidence_ids: Vec<Id>,
    pub source_ids: Vec<Id>,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MorphismPayload {
    #[serde(default)]
    pub added_cells: Vec<CaseCell>,
    #[serde(default)]
    pub added_relations: Vec<CaseRelation>,
    #[serde(default)]
    pub updated_cells: Vec<CaseCell>,
    #[serde(default)]
    pub updated_relations: Vec<CaseRelation>,
}

pub(crate) const GENESIS_CASE_SPACE_METADATA_KEY: &str = "genesis_case_space";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenesisCaseSpaceMaterialization {
    pub space_id: Id,
    pub projections: Vec<Projection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_policy_id: Option<Id>,
    pub metadata: Map<String, Value>,
    pub revision_metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MorphismApplyError {
    reason: String,
}

impl MorphismApplyError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for MorphismApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl std::error::Error for MorphismApplyError {}

pub(crate) fn write_genesis_materialization(
    case_space: &mut CaseSpace,
) -> Result<(), MorphismApplyError> {
    if case_space.morphism_log.len() != 1 {
        return Err(MorphismApplyError::new(
            "genesis materialization requires exactly one morphism log entry",
        ));
    }

    let payload = MorphismPayload {
        added_cells: case_space.case_cells.clone(),
        added_relations: case_space.case_relations.clone(),
        ..MorphismPayload::default()
    };
    let added_ids = payload
        .added_cells
        .iter()
        .map(|cell| cell.id.clone())
        .chain(
            payload
                .added_relations
                .iter()
                .map(|relation| relation.id.clone()),
        )
        .collect();
    let materialization = GenesisCaseSpaceMaterialization {
        space_id: case_space.space_id.clone(),
        projections: case_space.projections.clone(),
        close_policy_id: case_space.close_policy_id.clone(),
        metadata: case_space.metadata.clone(),
        revision_metadata: case_space.revision.metadata.clone(),
    };
    let genesis = &mut case_space.morphism_log[0].morphism;
    genesis.added_ids = added_ids;
    genesis.metadata.insert(
        "payload".to_owned(),
        serde_json::to_value(payload).map_err(|error| {
            MorphismApplyError::new(format!(
                "cannot serialize genesis morphism {} payload: {error}",
                genesis.morphism_id
            ))
        })?,
    );
    genesis.metadata.insert(
        GENESIS_CASE_SPACE_METADATA_KEY.to_owned(),
        serde_json::to_value(materialization).map_err(|error| {
            MorphismApplyError::new(format!(
                "cannot serialize genesis morphism {} case-space materialization: {error}",
                genesis.morphism_id
            ))
        })?,
    );
    Ok(())
}

pub(crate) fn genesis_case_space_materialization(
    morphism: &CaseMorphism,
) -> Result<GenesisCaseSpaceMaterialization, MorphismApplyError> {
    let value = morphism
        .metadata
        .get(GENESIS_CASE_SPACE_METADATA_KEY)
        .ok_or_else(|| {
            MorphismApplyError::new(format!(
                "genesis morphism {} is missing metadata.{GENESIS_CASE_SPACE_METADATA_KEY}",
                morphism.morphism_id
            ))
        })?;
    serde_json::from_value(value.clone()).map_err(|error| {
        MorphismApplyError::new(format!(
            "genesis morphism {} has malformed metadata.{GENESIS_CASE_SPACE_METADATA_KEY}: {error}",
            morphism.morphism_id
        ))
    })
}

pub fn apply_morphism(
    case_space: &mut CaseSpace,
    morphism: &CaseMorphism,
) -> Result<(), MorphismApplyError> {
    let is_genesis = morphism.source_revision_id.is_none()
        && case_space.case_cells.is_empty()
        && case_space.case_relations.is_empty()
        && case_space.morphism_log.is_empty();
    let payload = morphism_payload(morphism)?;
    let (declared_added, declared_updated) = validate_declared_morphism_ids(morphism)?;
    let (payload_added, payload_updated) = validate_payload_ids(morphism, &payload)?;
    require_matching_ids(
        morphism,
        "added_ids",
        &declared_added,
        "payload added_cells and added_relations",
        &payload_added,
    )?;
    require_matching_ids(
        morphism,
        "updated_ids",
        &declared_updated,
        "payload updated_cells and updated_relations",
        &payload_updated,
    )?;

    let mut next = case_space.clone();
    let mut known_ids = materialized_ids(&next);

    for cell in payload.added_cells {
        require_not_capability_administration(morphism, &cell, "add", is_genesis)?;
        if !known_ids.insert(cell.id.clone()) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} cannot add cell {}: id already exists",
                morphism.morphism_id, cell.id
            )));
        }
        next.case_cells.push(cell);
    }
    for relation in payload.added_relations {
        if !known_ids.insert(relation.id.clone()) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} cannot add relation {}: id already exists",
                morphism.morphism_id, relation.id
            )));
        }
        next.case_relations.push(relation);
    }

    for cell in payload.updated_cells {
        let existing = next
            .case_cells
            .iter_mut()
            .find(|candidate| candidate.id == cell.id)
            .ok_or_else(|| {
                MorphismApplyError::new(format!(
                    "morphism {} cannot update cell {}: cell does not exist",
                    morphism.morphism_id, cell.id
                ))
            })?;
        require_not_capability_administration(morphism, existing, "update", is_genesis)?;
        require_not_capability_administration(morphism, &cell, "update", is_genesis)?;
        require_immutable_cell_update_fields(morphism, existing, &cell)?;
        require_lifecycle_transition(morphism, &cell.id, existing.lifecycle, cell.lifecycle)?;
        *existing = cell;
    }
    for relation in payload.updated_relations {
        let existing = next
            .case_relations
            .iter_mut()
            .find(|candidate| candidate.id == relation.id)
            .ok_or_else(|| {
                MorphismApplyError::new(format!(
                    "morphism {} cannot update relation {}: relation does not exist",
                    morphism.morphism_id, relation.id
                ))
            })?;
        *existing = relation;
    }

    for id in &morphism.retired_ids {
        if let Some(cell) = next
            .case_cells
            .iter_mut()
            .find(|candidate| candidate.id == *id)
        {
            require_not_capability_administration(morphism, cell, "retire", is_genesis)?;
            require_lifecycle_transition(morphism, id, cell.lifecycle, CaseCellLifecycle::Retired)?;
            cell.lifecycle = CaseCellLifecycle::Retired;
        } else if let Some(index) = next
            .case_relations
            .iter()
            .position(|candidate| candidate.id == *id)
        {
            next.case_relations.remove(index);
        } else {
            return Err(MorphismApplyError::new(format!(
                "morphism {} cannot retire {}: id does not exist",
                morphism.morphism_id, id
            )));
        }
    }

    let cell_ids = next
        .case_cells
        .iter()
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    for relation in &next.case_relations {
        if !cell_ids.contains(&relation.from_id) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} leaves relation {} with missing from_id cell {}",
                morphism.morphism_id, relation.id, relation.from_id
            )));
        }
        if !cell_ids.contains(&relation.to_id) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} leaves relation {} with missing to_id cell {}",
                morphism.morphism_id, relation.id, relation.to_id
            )));
        }
    }

    *case_space = next;
    Ok(())
}

pub(crate) struct MorphismApplicationIndex {
    cell_positions: BTreeMap<Id, usize>,
    relation_positions: BTreeMap<Id, usize>,
}

impl MorphismApplicationIndex {
    pub(crate) fn new(case_space: &CaseSpace) -> Self {
        Self {
            cell_positions: case_space
                .case_cells
                .iter()
                .enumerate()
                .map(|(index, cell)| (cell.id.clone(), index))
                .collect(),
            relation_positions: case_space
                .case_relations
                .iter()
                .enumerate()
                .map(|(index, relation)| (relation.id.clone(), index))
                .collect(),
        }
    }

    fn contains(&self, id: &Id) -> bool {
        self.cell_positions.contains_key(id) || self.relation_positions.contains_key(id)
    }
}

pub(crate) fn apply_morphism_indexed(
    case_space: &mut CaseSpace,
    morphism: &CaseMorphism,
    index: &mut MorphismApplicationIndex,
) -> Result<(), MorphismApplyError> {
    let is_genesis = morphism.source_revision_id.is_none()
        && case_space.case_cells.is_empty()
        && case_space.case_relations.is_empty()
        && case_space.morphism_log.is_empty();
    let payload = morphism_payload(morphism)?;
    let (declared_added, declared_updated) = validate_declared_morphism_ids(morphism)?;
    let (payload_added, payload_updated) = validate_payload_ids(morphism, &payload)?;
    require_matching_ids(
        morphism,
        "added_ids",
        &declared_added,
        "payload added_cells and added_relations",
        &payload_added,
    )?;
    require_matching_ids(
        morphism,
        "updated_ids",
        &declared_updated,
        "payload updated_cells and updated_relations",
        &payload_updated,
    )?;

    for cell in &payload.added_cells {
        require_not_capability_administration(morphism, cell, "add", is_genesis)?;
        if index.contains(&cell.id) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} cannot add cell {}: id already exists",
                morphism.morphism_id, cell.id
            )));
        }
    }
    for relation in &payload.added_relations {
        if index.contains(&relation.id) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} cannot add relation {}: id already exists",
                morphism.morphism_id, relation.id
            )));
        }
    }
    for cell in &payload.updated_cells {
        let position = index.cell_positions.get(&cell.id).ok_or_else(|| {
            MorphismApplyError::new(format!(
                "morphism {} cannot update cell {}: cell does not exist",
                morphism.morphism_id, cell.id
            ))
        })?;
        let existing = &case_space.case_cells[*position];
        require_not_capability_administration(morphism, existing, "update", is_genesis)?;
        require_not_capability_administration(morphism, cell, "update", is_genesis)?;
        require_immutable_cell_update_fields(morphism, existing, cell)?;
        require_lifecycle_transition(morphism, &cell.id, existing.lifecycle, cell.lifecycle)?;
    }
    for relation in &payload.updated_relations {
        if !index.relation_positions.contains_key(&relation.id) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} cannot update relation {}: relation does not exist",
                morphism.morphism_id, relation.id
            )));
        }
    }
    for id in &morphism.retired_ids {
        if let Some(position) = index.cell_positions.get(id) {
            let cell = &case_space.case_cells[*position];
            require_not_capability_administration(morphism, cell, "retire", is_genesis)?;
            require_lifecycle_transition(morphism, id, cell.lifecycle, CaseCellLifecycle::Retired)?;
        } else if !index.relation_positions.contains_key(id) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} cannot retire {}: id does not exist",
                morphism.morphism_id, id
            )));
        }
    }

    let added_cell_ids = payload
        .added_cells
        .iter()
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    for relation in payload
        .added_relations
        .iter()
        .chain(&payload.updated_relations)
    {
        for (field, endpoint) in [("from_id", &relation.from_id), ("to_id", &relation.to_id)] {
            if !index.cell_positions.contains_key(endpoint) && !added_cell_ids.contains(endpoint) {
                return Err(MorphismApplyError::new(format!(
                    "morphism {} leaves relation {} with missing {field} cell {}",
                    morphism.morphism_id, relation.id, endpoint
                )));
            }
        }
    }

    for cell in payload.added_cells {
        index
            .cell_positions
            .insert(cell.id.clone(), case_space.case_cells.len());
        case_space.case_cells.push(cell);
    }
    for relation in payload.added_relations {
        index
            .relation_positions
            .insert(relation.id.clone(), case_space.case_relations.len());
        case_space.case_relations.push(relation);
    }
    for cell in payload.updated_cells {
        let position = index.cell_positions[&cell.id];
        case_space.case_cells[position] = cell;
    }
    for relation in payload.updated_relations {
        let position = index.relation_positions[&relation.id];
        case_space.case_relations[position] = relation;
    }

    let mut retired_relation_positions = Vec::new();
    for id in &morphism.retired_ids {
        if let Some(position) = index.cell_positions.get(id) {
            case_space.case_cells[*position].lifecycle = CaseCellLifecycle::Retired;
        } else {
            retired_relation_positions.push(index.relation_positions[id]);
        }
    }
    if !retired_relation_positions.is_empty() {
        retired_relation_positions.sort_unstable();
        for position in retired_relation_positions.into_iter().rev() {
            case_space.case_relations.remove(position);
        }
        index.relation_positions = case_space
            .case_relations
            .iter()
            .enumerate()
            .map(|(position, relation)| (relation.id.clone(), position))
            .collect();
    }
    Ok(())
}

fn morphism_payload(morphism: &CaseMorphism) -> Result<MorphismPayload, MorphismApplyError> {
    match morphism.metadata.get("payload") {
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
            MorphismApplyError::new(format!(
                "morphism {} has malformed metadata.payload: {error}",
                morphism.morphism_id
            ))
        }),
        None => Ok(MorphismPayload::default()),
    }
}

fn validate_declared_morphism_ids(
    morphism: &CaseMorphism,
) -> Result<(BTreeSet<Id>, BTreeSet<Id>), MorphismApplyError> {
    let mut all = BTreeSet::new();
    let added = collect_unique_ids(morphism, "added_ids", &morphism.added_ids, &mut all)?;
    let updated = collect_unique_ids(morphism, "updated_ids", &morphism.updated_ids, &mut all)?;
    collect_unique_ids(morphism, "retired_ids", &morphism.retired_ids, &mut all)?;
    Ok((added, updated))
}

fn collect_unique_ids(
    morphism: &CaseMorphism,
    list_name: &str,
    ids: &[Id],
    all: &mut BTreeSet<Id>,
) -> Result<BTreeSet<Id>, MorphismApplyError> {
    let mut list = BTreeSet::new();
    for id in ids {
        if !list.insert(id.clone()) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} contains duplicate id {} in {}",
                morphism.morphism_id, id, list_name
            )));
        }
        if !all.insert(id.clone()) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} contains duplicate id {} across added_ids, updated_ids, and retired_ids",
                morphism.morphism_id, id
            )));
        }
    }
    Ok(list)
}

fn validate_payload_ids(
    morphism: &CaseMorphism,
    payload: &MorphismPayload,
) -> Result<(BTreeSet<Id>, BTreeSet<Id>), MorphismApplyError> {
    let mut all = BTreeSet::new();
    let mut added = BTreeSet::new();
    let mut updated = BTreeSet::new();
    collect_payload_ids(
        morphism,
        "added_cells",
        payload.added_cells.iter().map(|cell| &cell.id),
        &mut added,
        &mut all,
    )?;
    collect_payload_ids(
        morphism,
        "added_relations",
        payload.added_relations.iter().map(|relation| &relation.id),
        &mut added,
        &mut all,
    )?;
    collect_payload_ids(
        morphism,
        "updated_cells",
        payload.updated_cells.iter().map(|cell| &cell.id),
        &mut updated,
        &mut all,
    )?;
    collect_payload_ids(
        morphism,
        "updated_relations",
        payload
            .updated_relations
            .iter()
            .map(|relation| &relation.id),
        &mut updated,
        &mut all,
    )?;
    Ok((added, updated))
}

fn collect_payload_ids<'a>(
    morphism: &CaseMorphism,
    list_name: &str,
    ids: impl Iterator<Item = &'a Id>,
    target: &mut BTreeSet<Id>,
    all: &mut BTreeSet<Id>,
) -> Result<(), MorphismApplyError> {
    for id in ids {
        if !all.insert(id.clone()) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} payload contains duplicate id {} in or across payload lists (at {})",
                morphism.morphism_id, id, list_name
            )));
        }
        target.insert(id.clone());
    }
    Ok(())
}

fn require_matching_ids(
    morphism: &CaseMorphism,
    declared_name: &str,
    declared: &BTreeSet<Id>,
    payload_name: &str,
    payload: &BTreeSet<Id>,
) -> Result<(), MorphismApplyError> {
    if declared == payload {
        return Ok(());
    }
    Err(MorphismApplyError::new(format!(
        "morphism {} {} [{}] do not match {} [{}]",
        morphism.morphism_id,
        declared_name,
        display_ids(declared),
        payload_name,
        display_ids(payload)
    )))
}

fn display_ids(ids: &BTreeSet<Id>) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn materialized_ids(case_space: &CaseSpace) -> BTreeSet<Id> {
    case_space
        .case_cells
        .iter()
        .map(|cell| cell.id.clone())
        .chain(
            case_space
                .case_relations
                .iter()
                .map(|relation| relation.id.clone()),
        )
        .collect()
}

fn require_not_capability_administration(
    morphism: &CaseMorphism,
    cell: &CaseCell,
    operation: &str,
    is_genesis: bool,
) -> Result<(), MorphismApplyError> {
    if cell.cell_type != CaseCellType::Custom("capability".to_owned()) || is_genesis {
        return Ok(());
    }
    Err(MorphismApplyError::new(format!(
        "morphism {} cannot {operation} capability cell {}: custom:capability cells are \
         administered only at lift/import time inside the declared source boundary",
        morphism.morphism_id, cell.id
    )))
}

fn require_immutable_cell_update_fields(
    morphism: &CaseMorphism,
    existing: &CaseCell,
    updated: &CaseCell,
) -> Result<(), MorphismApplyError> {
    if existing.cell_type != updated.cell_type {
        return Err(MorphismApplyError::new(format!(
            "morphism {} cannot update cell {}: cell_type is immutable ({} cannot become {})",
            morphism.morphism_id, existing.id, existing.cell_type, updated.cell_type
        )));
    }
    if existing.cell_type != CaseCellType::Evidence {
        return Ok(());
    }
    if existing.provenance != updated.provenance {
        return Err(MorphismApplyError::new(format!(
            "morphism {} cannot update evidence cell {}: provenance is immutable",
            morphism.morphism_id, existing.id
        )));
    }
    for key in [
        "evidence_boundary",
        "content_hash",
        "trace_id",
        "worker_report_id",
    ] {
        if existing.metadata.get(key) != updated.metadata.get(key) {
            return Err(MorphismApplyError::new(format!(
                "morphism {} cannot update evidence cell {}: metadata.{key} is immutable",
                morphism.morphism_id, existing.id
            )));
        }
    }
    Ok(())
}

fn require_lifecycle_transition(
    morphism: &CaseMorphism,
    cell_id: &Id,
    source: CaseCellLifecycle,
    target: CaseCellLifecycle,
) -> Result<(), MorphismApplyError> {
    if source.can_transition_to(target) {
        Ok(())
    } else {
        Err(MorphismApplyError::new(format!(
            "morphism {} cannot transition cell {} lifecycle from {:?} to {:?}",
            morphism.morphism_id, cell_id, source, target
        )))
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CaseMorphismType {
    Create,
    Update,
    Retire,
    Relate,
    Unrelate,
    Review,
    EvidenceAttach,
    CompletionAccept,
    CompletionReject,
    Projection,
    Migration,
    Close,
    Custom(String),
}

impl_custom_enum!(
    CaseMorphismType,
    {
        "create" => Create,
        "update" => Update,
        "retire" => Retire,
        "relate" => Relate,
        "unrelate" => Unrelate,
        "review" => Review,
        "evidence_attach" => EvidenceAttach,
        "completion_accept" => CompletionAccept,
        "completion_reject" => CompletionReject,
        "projection" => Projection,
        "migration" => Migration,
        "close" => Close
    }
);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MorphismLogEntry {
    pub schema: String,
    pub schema_version: u32,
    pub case_space_id: Id,
    pub sequence: u64,
    pub entry_id: Id,
    pub morphism_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_revision_id: Option<Id>,
    pub target_revision_id: Id,
    pub morphism: CaseMorphism,
    pub actor_id: Id,
    pub recorded_at: String,
    pub provenance: Provenance,
    pub source_ids: Vec<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_entry_hash: Option<String>,
    pub replay_checksum: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub revision_id: Id,
    pub case_space_id: Id,
    pub applied_entry_ids: Vec<Id>,
    pub applied_morphism_ids: Vec<Id>,
    pub checksum: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_revision_id: Option<Id>,
    pub created_at: String,
    pub source_ids: Vec<Id>,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    pub projection_id: Id,
    pub audience: ProjectionAudience,
    pub revision_id: Id,
    pub represented_cell_ids: Vec<Id>,
    pub represented_relation_ids: Vec<Id>,
    pub omitted_cell_ids: Vec<Id>,
    pub omitted_relation_ids: Vec<Id>,
    pub information_loss: Vec<ProjectionLoss>,
    pub allowed_operations: Vec<String>,
    pub source_ids: Vec<Id>,
    pub warnings: Vec<ProjectionWarning>,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionAudience {
    HumanReview,
    AiAgent,
    Audit,
    System,
    Migration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionLoss {
    pub description: String,
    pub represented_ids: Vec<Id>,
    pub omitted_ids: Vec<Id>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionWarning {
    HiddenBlocker,
    HiddenUnreviewedInference,
    HiddenCloseInvariantFailure,
    InformationLoss,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRecord {
    pub review_id: Id,
    pub target_ids: Vec<Id>,
    pub action: ReviewAction,
    pub outcome_review_status: ReviewStatus,
    pub reviewer_id: Id,
    pub reason: String,
    pub evidence_ids: Vec<Id>,
    pub source_ids: Vec<Id>,
    pub reviewed_at: String,
    pub provenance: Provenance,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
    Accept,
    Reject,
    Reopen,
    Waive,
    Defer,
    Supersede,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceBoundary {
    SourceBacked,
    Inferred,
    WorkerOutput,
    ReviewPromoted,
    Rejected,
    Contradicting,
}

impl EvidenceBoundary {
    pub(crate) fn from_metadata_value(value: Option<&str>) -> Self {
        match value {
            Some("source_backed" | "source_backed_evidence") => Self::SourceBacked,
            Some("inferred" | "ai_inference") => Self::Inferred,
            Some("worker_output") => Self::WorkerOutput,
            Some("review_promoted" | "review_promotion") => Self::ReviewPromoted,
            Some("rejected") => Self::Rejected,
            Some("contradicting") => Self::Contradicting,
            Some(_) | None => Self::Inferred,
        }
    }
}

impl From<EvidenceBoundary> for EvidenceTrustBoundary {
    fn from(boundary: EvidenceBoundary) -> Self {
        match boundary {
            EvidenceBoundary::SourceBacked => Self::SourceBacked,
            EvidenceBoundary::Inferred => Self::Inferred,
            EvidenceBoundary::WorkerOutput => Self::WorkerOutput,
            EvidenceBoundary::ReviewPromoted => Self::ReviewPromoted,
            EvidenceBoundary::Rejected => Self::Rejected,
            EvidenceBoundary::Contradicting => Self::Contradicting,
        }
    }
}

pub(crate) fn native_evidence_trust_input(
    cell: &CaseCell,
    latest_review_status: Option<ReviewStatus>,
) -> EvidenceTrustInput {
    EvidenceTrustInput {
        boundary: EvidenceBoundary::from_metadata_value(
            cell.metadata
                .get("evidence_boundary")
                .and_then(Value::as_str),
        )
        .into(),
        cell_review_status: cell.provenance.review_status,
        latest_review_status,
        has_source: !cell.source_ids.is_empty(),
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClosePolicy {
    pub policy_id: Id,
    pub required_goal_ids: Vec<Id>,
    pub required_projection_audiences: Vec<ProjectionAudience>,
    pub invariants: Vec<CloseInvariant>,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CloseInvariant {
    pub invariant_id: Id,
    pub invariant_type: CloseInvariantType,
    pub severity: Severity,
    pub description: String,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseInvariantType {
    NoHardObstructions,
    GoalsCovered,
    EvidenceAccepted,
    CompletionsReviewed,
    MorphismsReviewed,
    ProjectionsDiscloseLoss,
    BaseRevisionMatches,
    ReplayChecksumMatches,
    MigrationSourceRecorded,
    ValidationEvidenceNamed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CloseCheck {
    pub check_id: Id,
    pub case_space_id: Id,
    pub revision_id: Id,
    pub close_policy_id: Id,
    pub closable: bool,
    pub invariant_results: Vec<CloseInvariantResult>,
    pub completion_candidate_ids: Vec<Id>,
    pub evidence_ids: Vec<Id>,
    pub source_ids: Vec<Id>,
    pub provenance: Provenance,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CloseInvariantResult {
    pub invariant_id: Id,
    pub passed: bool,
    pub severity: Severity,
    pub witness_ids: Vec<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use higher_graphen_core::{Confidence, SourceKind, SourceRef};

    const NATIVE_EXAMPLE: &str =
        include_str!("../schemas/casegraphen/native.case.space.example.json");

    #[test]
    fn native_case_space_example_deserializes() {
        let space: CaseSpace =
            serde_json::from_str(NATIVE_EXAMPLE).expect("native case space example");

        assert_eq!(space.schema, NATIVE_CASE_SPACE_SCHEMA);
        assert_eq!(space.schema_version, NATIVE_CASE_SPACE_SCHEMA_VERSION);
        assert_eq!(space.case_cells.len(), 11);
        assert_eq!(space.case_relations.len(), 4);
        assert_eq!(space.morphism_log.len(), 1);
        assert_eq!(space.projections.len(), 2);
    }

    #[test]
    fn native_model_rejects_unknown_top_level_fields() {
        let mut value: Value =
            serde_json::from_str(NATIVE_EXAMPLE).expect("native case space example value");
        value["ready_cell_ids"] = Value::Array(Vec::new());

        assert!(serde_json::from_value::<CaseSpace>(value).is_err());
    }

    #[test]
    fn native_model_rejects_unknown_nested_fields() {
        let mut value: Value =
            serde_json::from_str(NATIVE_EXAMPLE).expect("native case space example value");
        value["case_cells"][0]["ready"] = Value::Bool(true);

        assert!(serde_json::from_value::<CaseSpace>(value).is_err());
    }

    #[test]
    fn review_and_evidence_boundaries_round_trip() {
        let review = ReviewRecord {
            review_id: id("review:accept-evidence"),
            target_ids: vec![id("evidence:source-backed-doc")],
            action: ReviewAction::Accept,
            outcome_review_status: ReviewStatus::Accepted,
            reviewer_id: id("reviewer:native-lead"),
            reason: "Source-backed evidence is sufficient.".to_owned(),
            evidence_ids: vec![id("evidence:source-backed-doc")],
            source_ids: vec![id("source:native-design")],
            reviewed_at: "2026-04-26T01:00:00Z".to_owned(),
            provenance: provenance(SourceKind::Human, ReviewStatus::Accepted),
            metadata: Map::new(),
        };
        let boundary = EvidenceBoundary::ReviewPromoted;

        let encoded_review = serde_json::to_string(&review).expect("serialize review");
        let encoded_boundary = serde_json::to_string(&boundary).expect("serialize boundary");

        assert_eq!(
            serde_json::from_str::<ReviewRecord>(&encoded_review).expect("deserialize review"),
            review
        );
        assert_eq!(
            serde_json::from_str::<EvidenceBoundary>(&encoded_boundary)
                .expect("deserialize evidence boundary"),
            boundary
        );
    }

    #[test]
    fn native_case_space_round_trips() {
        let space: CaseSpace =
            serde_json::from_str(NATIVE_EXAMPLE).expect("native case space example");
        let round_trip: CaseSpace =
            serde_json::from_str(&serde_json::to_string(&space).expect("serialize case space"))
                .expect("deserialize case space");

        assert_eq!(round_trip, space);
    }

    #[test]
    fn custom_extension_enums_require_non_empty_suffix() {
        assert_eq!(
            serde_json::from_value::<CaseCellType>(Value::String("custom:risk".to_owned()))
                .expect("custom cell type"),
            CaseCellType::Custom("risk".to_owned())
        );
        assert!(
            serde_json::from_value::<CaseMorphismType>(Value::String("custom:".to_owned()))
                .is_err()
        );
    }

    #[test]
    fn reducer_adds_a_cell_and_relation() {
        let mut space = fixture_space();
        let mut cell = space.case_cells[2].clone();
        cell.id = id("work:typed-reducer");
        cell.title = "Apply typed reducer".to_owned();
        cell.lifecycle = CaseCellLifecycle::Proposed;
        let mut relation = space.case_relations[0].clone();
        relation.id = id("relation:typed-reducer-depends-on-goal");
        relation.relation_type = CaseRelationType::DependsOn;
        relation.from_id = cell.id.clone();
        relation.to_id = id("goal:native-case-contract");
        relation.evidence_ids.clear();
        let mut morphism = fixture_morphism(&space);
        morphism.added_ids = vec![cell.id.clone(), relation.id.clone()];
        set_payload(
            &mut morphism,
            MorphismPayload {
                added_cells: vec![cell.clone()],
                added_relations: vec![relation.clone()],
                ..MorphismPayload::default()
            },
        );

        apply_morphism(&mut space, &morphism).expect("apply typed additions");

        assert!(space.case_cells.contains(&cell));
        assert!(space.case_relations.contains(&relation));
    }

    #[test]
    fn reducer_replaces_an_updated_cell_wholesale() {
        let mut space = fixture_space();
        let mut updated = space.case_cells[0].clone();
        updated.title = "Updated native case contract".to_owned();
        updated.summary = None;
        updated.lifecycle = CaseCellLifecycle::Waiting;
        let mut morphism = fixture_morphism(&space);
        morphism.updated_ids = vec![updated.id.clone()];
        set_payload(
            &mut morphism,
            MorphismPayload {
                updated_cells: vec![updated.clone()],
                ..MorphismPayload::default()
            },
        );

        apply_morphism(&mut space, &morphism).expect("apply cell update");

        assert_eq!(
            space
                .case_cells
                .iter()
                .find(|cell| cell.id == updated.id)
                .expect("updated cell"),
            &updated
        );
    }

    #[test]
    fn reducer_refuses_self_grant_updates_to_capability_cells() {
        let space = fixture_space();
        let mut capability = space
            .case_cells
            .iter()
            .find(|cell| cell.cell_type == CaseCellType::Custom("capability".to_owned()))
            .expect("fixture capability")
            .clone();
        capability.metadata.insert(
            "actor_ids".to_owned(),
            serde_json::json!(["actor:owner", "actor:attacker"]),
        );

        let error = rejected_cell_update(&space, capability);

        assert!(error.contains("capability cell capability:plan-review"));
        assert!(error
            .contains("administered only at lift/import time inside the declared source boundary"));
    }

    #[test]
    fn reducer_refuses_adding_or_retiring_capability_cells() {
        let space = fixture_space();
        let mut added_capability = space
            .case_cells
            .iter()
            .find(|cell| cell.cell_type == CaseCellType::Custom("capability".to_owned()))
            .expect("fixture capability")
            .clone();
        added_capability.id = id("capability:self-created");
        let mut add_morphism = fixture_morphism(&space);
        add_morphism.added_ids = vec![added_capability.id.clone()];
        set_payload(
            &mut add_morphism,
            MorphismPayload {
                added_cells: vec![added_capability],
                ..MorphismPayload::default()
            },
        );

        let add_error =
            apply_morphism(&mut space.clone(), &add_morphism).expect_err("capability add");
        assert!(add_error
            .to_string()
            .contains("cannot add capability cell capability:self-created"));

        let mut retire_morphism = fixture_morphism(&space);
        retire_morphism.retired_ids = vec![id("capability:plan-review")];
        let retire_error =
            apply_morphism(&mut space.clone(), &retire_morphism).expect_err("capability retire");
        assert!(retire_error
            .to_string()
            .contains("cannot retire capability cell capability:plan-review"));
    }

    #[test]
    fn reducer_refuses_cell_type_changes_on_update() {
        let space = fixture_space();
        let mut updated = space.case_cells[0].clone();
        updated.cell_type = CaseCellType::Work;

        let error = rejected_cell_update(&space, updated);

        assert!(error.contains("cell_type is immutable"));
        assert!(error.contains("goal cannot become work"));
    }

    #[test]
    fn reducer_refuses_security_relevant_evidence_rewrites() {
        let space = fixture_space();
        let evidence = space
            .case_cells
            .iter()
            .find(|cell| cell.cell_type == CaseCellType::Evidence)
            .expect("fixture evidence")
            .clone();

        let mut provenance = evidence.clone();
        provenance.provenance.review_status = ReviewStatus::Reviewed;
        assert!(rejected_cell_update(&space, provenance).contains("provenance is immutable"));

        for key in [
            "evidence_boundary",
            "content_hash",
            "trace_id",
            "worker_report_id",
        ] {
            let mut updated = evidence.clone();
            updated
                .metadata
                .insert(key.to_owned(), serde_json::json!("rewritten"));
            assert!(
                rejected_cell_update(&space, updated)
                    .contains(&format!("metadata.{key} is immutable")),
                "{key} should be immutable"
            );
        }
    }

    #[test]
    fn reducer_retires_a_cell_and_removes_a_relation() {
        let mut space = fixture_space();
        let cell_id = id("work:review-native-contract");
        let relation_id = id("relation:case-covers-goal");
        let mut morphism = fixture_morphism(&space);
        morphism.retired_ids = vec![cell_id.clone(), relation_id.clone()];

        apply_morphism(&mut space, &morphism).expect("apply retirements");

        assert_eq!(
            space
                .case_cells
                .iter()
                .find(|cell| cell.id == cell_id)
                .expect("retired cell")
                .lifecycle,
            CaseCellLifecycle::Retired
        );
        assert!(!space
            .case_relations
            .iter()
            .any(|relation| relation.id == relation_id));
    }

    #[test]
    fn reducer_rejects_payload_and_added_id_mismatch() {
        let mut space = fixture_space();
        let mut cell = space.case_cells[2].clone();
        cell.id = id("work:undeclared-payload");
        let mut morphism = fixture_morphism(&space);
        set_payload(
            &mut morphism,
            MorphismPayload {
                added_cells: vec![cell],
                ..MorphismPayload::default()
            },
        );

        let error = apply_morphism(&mut space, &morphism).expect_err("mismatched added ids");

        assert!(error.to_string().contains("added_ids"));
        assert!(error
            .to_string()
            .contains("payload added_cells and added_relations"));
    }

    #[test]
    fn reducer_rejects_unknown_updated_and_retired_ids() {
        let space = fixture_space();
        let mut unknown_update = space.case_cells[0].clone();
        unknown_update.id = id("work:missing-update");
        let mut update_morphism = fixture_morphism(&space);
        update_morphism.updated_ids = vec![unknown_update.id.clone()];
        set_payload(
            &mut update_morphism,
            MorphismPayload {
                updated_cells: vec![unknown_update],
                ..MorphismPayload::default()
            },
        );
        let update_error =
            apply_morphism(&mut space.clone(), &update_morphism).expect_err("unknown updated cell");
        assert!(update_error
            .to_string()
            .contains("cannot update cell work:missing-update: cell does not exist"));

        let mut retire_morphism = fixture_morphism(&space);
        retire_morphism.retired_ids = vec![id("relation:missing-retirement")];
        let retire_error =
            apply_morphism(&mut space.clone(), &retire_morphism).expect_err("unknown retired id");
        assert!(retire_error
            .to_string()
            .contains("cannot retire relation:missing-retirement: id does not exist"));
    }

    #[test]
    fn reducer_rejects_duplicate_payload_ids() {
        let mut space = fixture_space();
        let mut cell = space.case_cells[2].clone();
        cell.id = id("work:duplicate-payload");
        let mut morphism = fixture_morphism(&space);
        morphism.added_ids = vec![cell.id.clone()];
        set_payload(
            &mut morphism,
            MorphismPayload {
                added_cells: vec![cell.clone(), cell],
                ..MorphismPayload::default()
            },
        );

        let error = apply_morphism(&mut space, &morphism).expect_err("duplicate payload id");

        assert!(error
            .to_string()
            .contains("duplicate id work:duplicate-payload"));
    }

    #[test]
    fn reducer_rejects_a_relation_to_a_missing_cell() {
        let mut space = fixture_space();
        let mut relation = space.case_relations[0].clone();
        relation.id = id("relation:missing-endpoint");
        relation.from_id = id("work:not-present");
        let mut morphism = fixture_morphism(&space);
        morphism.added_ids = vec![relation.id.clone()];
        set_payload(
            &mut morphism,
            MorphismPayload {
                added_relations: vec![relation],
                ..MorphismPayload::default()
            },
        );

        let error = apply_morphism(&mut space, &morphism).expect_err("missing relation endpoint");

        assert!(error.to_string().contains(
            "relation relation:missing-endpoint with missing from_id cell work:not-present"
        ));
    }

    #[test]
    fn reducer_rejects_an_illegal_lifecycle_transition() {
        let mut space = fixture_space();
        let mut updated = space.case_cells[0].clone();
        updated.lifecycle = CaseCellLifecycle::Accepted;
        let mut morphism = fixture_morphism(&space);
        morphism.updated_ids = vec![updated.id.clone()];
        set_payload(
            &mut morphism,
            MorphismPayload {
                updated_cells: vec![updated],
                ..MorphismPayload::default()
            },
        );

        let error =
            apply_morphism(&mut space, &morphism).expect_err("illegal lifecycle transition");

        assert!(error.to_string().contains(
            "cannot transition cell goal:native-case-contract lifecycle from Active to Accepted"
        ));
    }

    #[test]
    fn reducer_accepts_a_metadata_only_morphism() {
        let mut space = fixture_space();
        let before = space.clone();
        let morphism = fixture_morphism(&space);

        apply_morphism(&mut space, &morphism).expect("metadata-only morphism");

        assert_eq!(space, before);
    }

    #[test]
    fn reducer_rejects_a_malformed_payload() {
        let mut space = fixture_space();
        let mut morphism = fixture_morphism(&space);
        morphism
            .metadata
            .insert("payload".to_owned(), serde_json::json!({"unknown": []}));

        let error = apply_morphism(&mut space, &morphism).expect_err("malformed payload");

        assert!(error.to_string().contains("malformed metadata.payload"));
        assert!(error.to_string().contains("unknown field"));
    }

    fn fixture_space() -> CaseSpace {
        serde_json::from_str(NATIVE_EXAMPLE).expect("native case space example")
    }

    fn fixture_morphism(space: &CaseSpace) -> CaseMorphism {
        CaseMorphism {
            morphism_id: id("morphism:typed-reducer-test"),
            morphism_type: CaseMorphismType::Update,
            source_revision_id: Some(space.revision.revision_id.clone()),
            target_revision_id: id("revision:typed-reducer-test"),
            added_ids: Vec::new(),
            updated_ids: Vec::new(),
            retired_ids: Vec::new(),
            preserved_ids: Vec::new(),
            violated_invariant_ids: Vec::new(),
            review_status: ReviewStatus::Accepted,
            evidence_ids: Vec::new(),
            source_ids: Vec::new(),
            metadata: Map::new(),
        }
    }

    fn set_payload(morphism: &mut CaseMorphism, payload: MorphismPayload) {
        morphism.metadata.insert(
            "payload".to_owned(),
            serde_json::to_value(payload).expect("serialize morphism payload"),
        );
    }

    fn rejected_cell_update(space: &CaseSpace, updated: CaseCell) -> String {
        let mut morphism = fixture_morphism(space);
        morphism.updated_ids = vec![updated.id.clone()];
        set_payload(
            &mut morphism,
            MorphismPayload {
                updated_cells: vec![updated],
                ..MorphismPayload::default()
            },
        );
        apply_morphism(&mut space.clone(), &morphism)
            .expect_err("cell update should be rejected")
            .to_string()
    }

    fn provenance(kind: SourceKind, review_status: ReviewStatus) -> Provenance {
        Provenance::new(
            SourceRef::new(kind),
            Confidence::new(1.0).expect("confidence"),
        )
        .with_review_status(review_status)
    }

    fn id(value: &str) -> Id {
        Id::new(value).expect("fixture id")
    }
}

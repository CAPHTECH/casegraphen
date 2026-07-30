//! Shared topology diagnostics for CaseGraphen data models.

use crate::native_model;
use higher_graphen_core::{CoreError, Id};
use higher_graphen_structure::space::{Dimension, GraphAnalyticsReport};
use higher_graphen_structure::topology::TopologySummary;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[path = "topology_higher_order.rs"]
mod topology_higher_order;
#[path = "topology_lift.rs"]
mod topology_lift;

pub use self::topology_higher_order::{
    HigherOrderFiltrationSource, HigherOrderFiltrationStageSource, HigherOrderIntervalSummary,
    HigherOrderTopologyReport, HigherOrderTopologySummary,
};
pub use self::topology_lift::{SkippedRelationMapping, SourceCellMapping, TopologyLiftSummary};

use self::topology_higher_order::HigherOrderFiltrationInput;
use self::topology_lift::{cell_id, native_lift_builder, LiftBuilder};

/// Topology report for a lifted CaseGraphen graph.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseTopologyReport {
    /// Space summarized by the topology engine.
    pub space_id: Id,
    /// Complex summarized by the topology engine.
    pub complex_id: Id,
    /// Shared finite topology summary over the lifted complex.
    pub topology: TopologySummary,
    /// Finite graph analytics over the lifted incidence view.
    pub graph_analytics: GraphAnalyticsReport,
    /// Deterministic mapping from source records to generated cells.
    pub source_mapping: TopologyLiftSummary,
    /// Optional higher-order persistence summary for opt-in CLI diagnostics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub higher_order: Option<HigherOrderTopologyReport>,
}

/// Options for opt-in higher-order topology diagnostics.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyReportOptions {
    /// Whether to include higher-order persistence diagnostics.
    pub include_higher_order: bool,
    /// Optional maximum cell dimension included in the filtration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_dimension: Option<Dimension>,
    /// Minimum interval lifetime in stage steps for persistent interval reporting.
    pub min_persistence_stages: usize,
}

impl TopologyReportOptions {
    /// Returns options that emit only the baseline static topology report.
    #[must_use]
    pub fn baseline() -> Self {
        Self::default()
    }

    /// Returns options that include opt-in higher-order persistence diagnostics.
    #[must_use]
    pub fn higher_order(max_dimension: Option<Dimension>, min_persistence_stages: usize) -> Self {
        Self {
            include_higher_order: true,
            max_dimension,
            min_persistence_stages,
        }
    }
}

/// File-to-file topology diff between two lifted CaseGraphen topology reports.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyDiffReport {
    /// Space summarized by the left topology report.
    pub left_space_id: Id,
    /// Space summarized by the right topology report.
    pub right_space_id: Id,
    /// Complex summarized by the left topology report.
    pub left_complex_id: Id,
    /// Complex summarized by the right topology report.
    pub right_complex_id: Id,
    /// Scalar topology count changes from left to right.
    pub scalar_deltas: TopologyScalarDeltas,
    /// Source record additions and removals derived from source mappings.
    pub source_mapping: TopologySourceMappingDiff,
    /// Higher-order summary diff when both reports include higher-order summaries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub higher_order: Option<HigherOrderTopologyDiff>,
}

/// Scalar topology count changes from left to right.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyScalarDeltas {
    pub vertex_count: ScalarDelta,
    pub graph_edge_count: ScalarDelta,
    pub component_count: ScalarDelta,
    pub first_betti_number: ScalarDelta,
    pub simple_hole_count: ScalarDelta,
    pub euler_characteristic: ScalarDelta,
}

/// Numeric delta with left and right values retained for review.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarDelta {
    pub left: i64,
    pub right: i64,
    pub delta: i64,
}

/// Source record additions and removals derived from topology lift mappings.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopologySourceMappingDiff {
    pub added_source_node_ids: Vec<Id>,
    pub removed_source_node_ids: Vec<Id>,
    pub added_source_relation_ids: Vec<Id>,
    pub removed_source_relation_ids: Vec<Id>,
}

/// Compact higher-order topology summary diff.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HigherOrderTopologyDiff {
    pub interval_count_by_dimension: BTreeMap<Dimension, ScalarDelta>,
    pub open_interval_count_by_dimension: BTreeMap<Dimension, ScalarDelta>,
    pub persistent_interval_count_by_dimension: BTreeMap<Dimension, ScalarDelta>,
    pub max_betti_rank: ScalarDelta,
    pub max_betti_rank_dimension: OptionalDimensionDelta,
    pub highest_nonzero_betti_dimension: OptionalDimensionDelta,
}

/// Optional dimension change with left and right values retained for review.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionalDimensionDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Dimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Dimension>,
    pub changed: bool,
}

/// Error returned while building or summarizing a topology report.
pub type TopologyReportError = CoreError;

/// Lifts a native case space into a finite complex and summarizes it.
pub fn native_case_topology(
    case_space: &native_model::CaseSpace,
) -> Result<CaseTopologyReport, TopologyReportError> {
    native_case_topology_with_options(case_space, TopologyReportOptions::baseline())
}

/// Lifts a native case space into a finite complex and summarizes it.
pub fn native_case_topology_with_options(
    case_space: &native_model::CaseSpace,
    options: TopologyReportOptions,
) -> Result<CaseTopologyReport, TopologyReportError> {
    let mut lift = LiftBuilder::new(
        case_space.space_id.clone(),
        cell_id("complex", "native_case_space", &case_space.case_space_id)?,
        "CaseGraphen native case topology",
    )?;

    for cell in &case_space.case_cells {
        lift.add_node("case_cell", &cell.id, &cell.title)?;
    }
    for projection in &case_space.projections {
        lift.add_node("projection", &projection.projection_id, "native projection")?;
    }
    lift.add_node(
        "revision",
        &case_space.revision.revision_id,
        "native revision",
    )?;
    for entry in &case_space.morphism_log {
        lift.add_node("morphism_log_entry", &entry.entry_id, "morphism log entry")?;
        lift.add_node("morphism", &entry.morphism_id, "morphism")?;
    }
    for relation in &case_space.case_relations {
        lift.add_relation(
            "case_relation",
            &relation.id,
            &relation.relation_type.to_string(),
            &relation.from_id,
            &relation.to_id,
        )?;
    }

    lift.finish(options)
}

/// Lifts a native case space and uses its morphism log as the higher-order filtration source.
pub fn native_case_topology_with_history(
    case_space: &native_model::CaseSpace,
    history: &[native_model::MorphismLogEntry],
    options: TopologyReportOptions,
) -> Result<CaseTopologyReport, TopologyReportError> {
    let lift = native_lift_builder(case_space)?;
    lift.finish_with_filtration(
        options,
        HigherOrderFiltrationInput::NativeMorphismLog(history),
    )
}

/// Compares two topology reports using deterministic scalar and source-mapping deltas.
#[must_use]
pub fn topology_diff(left: &CaseTopologyReport, right: &CaseTopologyReport) -> TopologyDiffReport {
    TopologyDiffReport {
        left_space_id: left.space_id.clone(),
        right_space_id: right.space_id.clone(),
        left_complex_id: left.complex_id.clone(),
        right_complex_id: right.complex_id.clone(),
        scalar_deltas: TopologyScalarDeltas {
            vertex_count: usize_delta(left.topology.vertex_count, right.topology.vertex_count),
            graph_edge_count: usize_delta(
                left.topology.graph_edge_count,
                right.topology.graph_edge_count,
            ),
            component_count: usize_delta(
                left.topology.component_count,
                right.topology.component_count,
            ),
            first_betti_number: usize_delta(
                left.topology.first_betti_number,
                right.topology.first_betti_number,
            ),
            simple_hole_count: usize_delta(
                left.topology.simple_hole_count,
                right.topology.simple_hole_count,
            ),
            euler_characteristic: i64_delta(
                left.topology.homology.euler_characteristic,
                right.topology.homology.euler_characteristic,
            ),
        },
        source_mapping: source_mapping_diff(&left.source_mapping, &right.source_mapping),
        higher_order: higher_order_diff(left, right),
    }
}

fn source_mapping_diff(
    left: &TopologyLiftSummary,
    right: &TopologyLiftSummary,
) -> TopologySourceMappingDiff {
    let (added_source_node_ids, removed_source_node_ids) = set_diff(
        &mapped_source_ids(&left.nodes),
        &mapped_source_ids(&right.nodes),
    );
    let (added_source_relation_ids, removed_source_relation_ids) = set_diff(
        &mapped_source_ids(&left.relations),
        &mapped_source_ids(&right.relations),
    );

    TopologySourceMappingDiff {
        added_source_node_ids,
        removed_source_node_ids,
        added_source_relation_ids,
        removed_source_relation_ids,
    }
}

fn higher_order_diff(
    left: &CaseTopologyReport,
    right: &CaseTopologyReport,
) -> Option<HigherOrderTopologyDiff> {
    let left_summary = left.higher_order.as_ref()?.summary.as_ref()?;
    let right_summary = right.higher_order.as_ref()?.summary.as_ref()?;

    Some(HigherOrderTopologyDiff {
        interval_count_by_dimension: count_map_delta(
            &left_summary.interval_count_by_dimension,
            &right_summary.interval_count_by_dimension,
        ),
        open_interval_count_by_dimension: count_map_delta(
            &left_summary.open_interval_count_by_dimension,
            &right_summary.open_interval_count_by_dimension,
        ),
        persistent_interval_count_by_dimension: count_map_delta(
            &left_summary.persistent_interval_count_by_dimension,
            &right_summary.persistent_interval_count_by_dimension,
        ),
        max_betti_rank: usize_delta(left_summary.max_betti_rank, right_summary.max_betti_rank),
        max_betti_rank_dimension: optional_dimension_delta(
            left_summary.max_betti_rank_dimension,
            right_summary.max_betti_rank_dimension,
        ),
        highest_nonzero_betti_dimension: optional_dimension_delta(
            left_summary.highest_nonzero_betti_dimension,
            right_summary.highest_nonzero_betti_dimension,
        ),
    })
}

fn mapped_source_ids(mappings: &[SourceCellMapping]) -> BTreeSet<Id> {
    mappings
        .iter()
        .map(|mapping| mapping.source_id.clone())
        .collect()
}

fn set_diff(left: &BTreeSet<Id>, right: &BTreeSet<Id>) -> (Vec<Id>, Vec<Id>) {
    (
        right.difference(left).cloned().collect(),
        left.difference(right).cloned().collect(),
    )
}

fn count_map_delta(
    left: &BTreeMap<Dimension, usize>,
    right: &BTreeMap<Dimension, usize>,
) -> BTreeMap<Dimension, ScalarDelta> {
    left.keys()
        .chain(right.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|dimension| {
            (
                dimension,
                usize_delta(
                    left.get(&dimension).copied().unwrap_or_default(),
                    right.get(&dimension).copied().unwrap_or_default(),
                ),
            )
        })
        .collect()
}

fn usize_delta(left: usize, right: usize) -> ScalarDelta {
    i64_delta(left as i64, right as i64)
}

fn i64_delta(left: i64, right: i64) -> ScalarDelta {
    ScalarDelta {
        left,
        right,
        delta: right - left,
    }
}

fn optional_dimension_delta(
    left: Option<Dimension>,
    right: Option<Dimension>,
) -> OptionalDimensionDelta {
    OptionalDimensionDelta {
        left,
        right,
        changed: left != right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NATIVE_EXAMPLE: &str =
        include_str!("../schemas/casegraphen/native.case.space.example.json");

    #[test]
    fn native_case_topology_serializes_homology() {
        let case_space: native_model::CaseSpace =
            serde_json::from_str(NATIVE_EXAMPLE).expect("native case space example");
        let report = native_case_topology(&case_space).expect("native case topology");

        assert_eq!(report.space_id, case_space.space_id);
        assert!(report.topology.homology.betti_number(0) > 0);
        assert_eq!(report.topology.homology.betti_number(1), 0);
        assert!(!report.source_mapping.nodes.is_empty());
        assert!(!report.source_mapping.relations.is_empty());

        serde_json::to_value(&report).expect("serialize native topology");
    }
}

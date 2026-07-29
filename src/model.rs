use higher_graphen_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PROJECTION_SCHEMA: &str = "highergraphen.case.projection.v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionDefinition {
    pub schema: String,
    pub projection_id: Id,
    pub audience: String,
    #[serde(default = "default_include_sources")]
    pub include_sources: bool,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

fn default_include_sources() -> bool {
    true
}

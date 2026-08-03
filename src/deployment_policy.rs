//! Content binding for the deployment policies that accompany a topology.

use crate::execution_topology::ExecutionTopology;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const DEPLOYMENT_POLICY_MANIFEST_SCHEMA: &str =
    "casegraphen.experimental.deployment_policy_manifest.v0";
pub const DEPLOYMENT_POLICY_MANIFEST_SCHEMA_VERSION: u32 = 0;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyContentBinding {
    pub policy_id: String,
    pub content_hash: String,
}

/// Canonical list of every policy document authorized with a topology.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentPolicyManifest {
    pub schema: String,
    pub schema_version: u32,
    pub topology_id: String,
    pub topology_content_hash: String,
    pub verification_policies: Vec<PolicyContentBinding>,
    pub budget_policies: Vec<PolicyContentBinding>,
    pub expansion_policies: Vec<PolicyContentBinding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyManifestFinding {
    pub code: String,
    pub location: String,
    pub detail: String,
}

pub fn deployment_policy_manifest(
    topology: &ExecutionTopology,
    topology_content_hash: &str,
    verification_policies: &BTreeMap<String, Value>,
    budget_policies: &BTreeMap<String, Value>,
    expansion_policies: &BTreeMap<String, Value>,
) -> DeploymentPolicyManifest {
    DeploymentPolicyManifest {
        schema: DEPLOYMENT_POLICY_MANIFEST_SCHEMA.to_owned(),
        schema_version: DEPLOYMENT_POLICY_MANIFEST_SCHEMA_VERSION,
        topology_id: topology.topology_id.clone(),
        topology_content_hash: topology_content_hash.to_owned(),
        verification_policies: content_bindings(verification_policies),
        budget_policies: content_bindings(budget_policies),
        expansion_policies: content_bindings(expansion_policies),
    }
}

pub fn deployment_policy_manifest_content_hash(
    manifest: &DeploymentPolicyManifest,
) -> Result<String, serde_json::Error> {
    let mut canonical = manifest.clone();
    for bindings in [
        &mut canonical.verification_policies,
        &mut canonical.budget_policies,
        &mut canonical.expansion_policies,
    ] {
        bindings.sort_by(|left, right| {
            (&left.policy_id, &left.content_hash).cmp(&(&right.policy_id, &right.content_hash))
        });
    }
    Ok(crate::native_hash::sha256_hex(&canonical_json_bytes(
        &canonical,
    )?))
}

pub fn validate_deployment_policy_manifest(
    topology: &ExecutionTopology,
    topology_content_hash: &str,
    manifest: &DeploymentPolicyManifest,
) -> Vec<PolicyManifestFinding> {
    let mut findings = Vec::new();
    if manifest.schema != DEPLOYMENT_POLICY_MANIFEST_SCHEMA
        || manifest.schema_version != DEPLOYMENT_POLICY_MANIFEST_SCHEMA_VERSION
    {
        findings.push(finding(
            "invalid_policy_manifest_schema",
            "$.schema",
            "deployment policy manifest schema identity/version is unsupported",
        ));
    }
    if manifest.topology_id != topology.topology_id {
        findings.push(finding(
            "policy_manifest_topology_mismatch",
            "$.topology_id",
            "policy manifest belongs to a different topology",
        ));
    }
    if manifest.topology_content_hash != topology_content_hash {
        findings.push(finding(
            "policy_manifest_topology_hash_mismatch",
            "$.topology_content_hash",
            "policy manifest is bound to different topology bytes",
        ));
    }
    for (kind, expected, supplied) in [
        (
            "verification",
            &topology.verification_policy_ids,
            &manifest.verification_policies,
        ),
        (
            "budget",
            &topology.budget_policy_ids,
            &manifest.budget_policies,
        ),
        (
            "expansion",
            &topology.expansion_policy_ids,
            &manifest.expansion_policies,
        ),
    ] {
        validate_bindings(kind, expected, supplied, &mut findings);
    }
    findings.sort_by(|left, right| {
        (&left.code, &left.location, &left.detail).cmp(&(
            &right.code,
            &right.location,
            &right.detail,
        ))
    });
    findings
}

fn content_bindings(documents: &BTreeMap<String, Value>) -> Vec<PolicyContentBinding> {
    documents
        .iter()
        .map(|(policy_id, document)| PolicyContentBinding {
            policy_id: policy_id.clone(),
            content_hash: crate::native_hash::sha256_hex(
                &canonical_value_bytes(document).expect("JSON values serialize canonically"),
            ),
        })
        .collect()
}

fn validate_bindings(
    kind: &str,
    expected: &[String],
    supplied: &[PolicyContentBinding],
    findings: &mut Vec<PolicyManifestFinding>,
) {
    let expected_set = expected.iter().cloned().collect::<BTreeSet<_>>();
    if expected_set.len() != expected.len() {
        findings.push(finding(
            format!("duplicate_{kind}_policy_reference"),
            format!("$.{kind}_policies"),
            "topology contains a duplicate policy reference",
        ));
    }
    let mut supplied_set = BTreeSet::new();
    for binding in supplied {
        if !supplied_set.insert(binding.policy_id.clone()) {
            findings.push(finding(
                format!("duplicate_{kind}_policy_binding"),
                format!("$.{kind}_policies"),
                format!("policy {} is bound more than once", binding.policy_id),
            ));
        }
        if !is_sha256(&binding.content_hash) {
            findings.push(finding(
                format!("invalid_{kind}_policy_hash"),
                format!("$.{kind}_policies"),
                format!(
                    "policy {} content_hash is not lowercase sha256",
                    binding.policy_id
                ),
            ));
        }
    }
    for missing in expected_set.difference(&supplied_set) {
        findings.push(finding(
            format!("missing_{kind}_policy_binding"),
            format!("$.{kind}_policies"),
            format!("topology policy {missing} is not content-bound"),
        ));
    }
    for extra in supplied_set.difference(&expected_set) {
        findings.push(finding(
            format!("undeclared_{kind}_policy_binding"),
            format!("$.{kind}_policies"),
            format!("manifest binds undeclared policy {extra}"),
        ));
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    canonical_value_bytes(&value)
}

fn canonical_value_bytes(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&canonical_value(value))
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), canonical_value(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn finding(
    code: impl Into<String>,
    location: impl Into<String>,
    detail: impl Into<String>,
) -> PolicyManifestFinding {
    PolicyManifestFinding {
        code: code.into(),
        location: location.into(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_topology::execution_topology_content_hash;
    use serde_json::json;

    fn topology() -> ExecutionTopology {
        serde_json::from_str(include_str!(
            "../schemas/experimental/execution.topology.file-review.example.json"
        ))
        .expect("topology fixture")
    }

    fn documents(
        topology: &ExecutionTopology,
    ) -> (
        BTreeMap<String, Value>,
        BTreeMap<String, Value>,
        BTreeMap<String, Value>,
    ) {
        (
            topology
                .verification_policy_ids
                .iter()
                .map(|id| (id.clone(), json!({"verification_policy_id": id})))
                .collect(),
            topology
                .budget_policy_ids
                .iter()
                .map(|id| (id.clone(), json!({"policy_id": id})))
                .collect(),
            BTreeMap::new(),
        )
    }

    #[test]
    fn manifest_binds_policy_content_not_only_ids() {
        let topology = topology();
        let topology_hash = execution_topology_content_hash(&topology).unwrap();
        let (verification, mut budget, expansion) = documents(&topology);
        let first = deployment_policy_manifest(
            &topology,
            &topology_hash,
            &verification,
            &budget,
            &expansion,
        );
        budget.get_mut("budget:small").unwrap()["max_cost"] = json!(11);
        let substituted = deployment_policy_manifest(
            &topology,
            &topology_hash,
            &verification,
            &budget,
            &expansion,
        );
        assert_ne!(
            deployment_policy_manifest_content_hash(&first).unwrap(),
            deployment_policy_manifest_content_hash(&substituted).unwrap()
        );
    }

    #[test]
    fn manifest_hash_canonicalizes_binding_order() {
        let topology = topology();
        let topology_hash = execution_topology_content_hash(&topology).unwrap();
        let (verification, budget, expansion) = documents(&topology);
        let mut manifest = deployment_policy_manifest(
            &topology,
            &topology_hash,
            &verification,
            &budget,
            &expansion,
        );
        manifest.budget_policies.push(PolicyContentBinding {
            policy_id: "budget:other".to_owned(),
            content_hash: "d".repeat(64),
        });
        let hash = deployment_policy_manifest_content_hash(&manifest).unwrap();
        manifest.budget_policies.reverse();
        assert_eq!(
            hash,
            deployment_policy_manifest_content_hash(&manifest).unwrap()
        );
    }

    #[test]
    fn manifest_validation_refuses_missing_extra_duplicate_and_cross_topology() {
        let topology = topology();
        let topology_hash = execution_topology_content_hash(&topology).unwrap();
        let (verification, budget, expansion) = documents(&topology);
        let mut manifest = deployment_policy_manifest(
            &topology,
            &topology_hash,
            &verification,
            &budget,
            &expansion,
        );
        manifest.verification_policies.clear();
        manifest.budget_policies.push(PolicyContentBinding {
            policy_id: "budget:extra".to_owned(),
            content_hash: "a".repeat(64),
        });
        manifest
            .budget_policies
            .push(manifest.budget_policies[0].clone());
        manifest.topology_id = "topology:other".to_owned();
        let codes = validate_deployment_policy_manifest(&topology, &topology_hash, &manifest)
            .into_iter()
            .map(|finding| finding.code)
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("missing_verification_policy_binding"));
        assert!(codes.contains("undeclared_budget_policy_binding"));
        assert!(codes.contains("duplicate_budget_policy_binding"));
        assert!(codes.contains("policy_manifest_topology_mismatch"));
    }
}

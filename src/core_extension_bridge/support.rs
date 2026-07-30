use super::CaseGraphenCoreExtensions;
use higher_graphen_core::{
    Confidence, Id, PayloadRef, Provenance, ReviewStatus, SourceKind, SourceRef,
};
use serde_json::{Map, Value};

const EXTENSIONS_METADATA_KEY: &str = "higher_graphen_extensions";

pub(crate) fn metadata_extensions(metadata: &Map<String, Value>) -> CaseGraphenCoreExtensions {
    metadata
        .get(EXTENSIONS_METADATA_KEY)
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .expect("metadata.higher_graphen_extensions must match CaseGraphenCoreExtensions")
        .unwrap_or_default()
}

pub(crate) fn generated_provenance(
    uri: String,
    title: &str,
    review_status: ReviewStatus,
    score: f64,
) -> Provenance {
    Provenance::new(
        SourceRef::new(SourceKind::Code)
            .with_uri(uri)
            .expect("generated source uri is valid")
            .with_title(title)
            .expect("generated source title is valid"),
        confidence(score),
    )
    .with_review_status(review_status)
    .with_extraction_method("casegraphen-core-extension-bridge")
    .expect("generated extraction method is valid")
}

pub(crate) fn payload_ref(kind: &str, uri: String) -> PayloadRef {
    PayloadRef::new(kind, uri).expect("generated payload ref is valid")
}

pub(crate) fn source_uri(namespace: &str, root: &Id, segment: &str, id: &Id) -> String {
    format!(
        "casegraphen://{namespace}/{}/{segment}/{}",
        uri_token(root),
        uri_token(id)
    )
}

pub(crate) fn generated_id(prefix: &str, parts: &[&str]) -> Id {
    let suffix = parts
        .iter()
        .map(|part| sanitize_id_part(part))
        .collect::<Vec<_>>()
        .join(":");
    Id::new(format!("{prefix}:{suffix}")).expect("generated id is valid")
}

pub(crate) fn sanitize_id_part(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, ':' | '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

pub(crate) fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("static confidence is valid")
}

fn uri_token(id: &Id) -> String {
    sanitize_id_part(id.as_str()).replace(':', "/")
}

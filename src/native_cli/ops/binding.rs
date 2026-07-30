use super::{
    io::{read_json, write_json},
    relative_store_path, report, NativeCliError,
};
use crate::exec::binding::{
    resolve_worker_binding_identity, validate_worker_binding, worker_binding_content_hash,
    WorkerBinding,
};
use higher_graphen_core::Id;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::super::path_helpers::path_segment;

const BINDING_DIRECTORY: &str = "bindings";

pub(in crate::native_cli) fn binding_register(
    store: &Path,
    input: &Path,
) -> Result<Value, NativeCliError> {
    let mut binding: WorkerBinding = serde_json::from_value(read_json(input)?)?;
    let identity = resolve_worker_binding_identity(&binding)
        .map_err(|error| NativeCliError::invalid(format!("{}: {error}", input.display())))?;
    binding.resolved_command_path = identity.resolved_command_path;
    binding.resolved_working_directory = identity.resolved_working_directory;
    binding.command_content_hash = identity.command_content_hash;
    validate_worker_binding(&binding)
        .map_err(|error| NativeCliError::invalid(format!("{}: {error}", input.display())))?;
    let path = binding_path(store, &binding.binding_id);
    if path.exists() {
        return Err(NativeCliError::invalid(format!(
            "worker binding {} already exists at {}",
            binding.binding_id,
            path.display()
        )));
    }
    let content_hash = worker_binding_content_hash(&binding)?;
    write_json(&path, &serde_json::to_value(&binding)?)?;
    Ok(report(
        "casegraphen binding register",
        json!({
            "binding_status": "registered",
            "binding_path": relative_store_path(store, &path),
            "binding_content_hash": content_hash,
            "binding": binding,
        }),
    ))
}

pub(super) fn read_registered_worker_binding(
    store: &Path,
    binding_id: &Id,
) -> Result<WorkerBinding, NativeCliError> {
    let path = binding_path(store, binding_id);
    if !path.exists() {
        return Err(NativeCliError::invalid(format!(
            "worker binding {binding_id} is not registered at {}",
            path.display()
        )));
    }
    let binding = read_worker_binding_file(&path)?;
    if binding.binding_id != *binding_id {
        return Err(NativeCliError::invalid(format!(
            "{}: worker binding id {} does not match requested {binding_id}",
            path.display(),
            binding.binding_id
        )));
    }
    Ok(binding)
}

pub(super) fn binding_path(store: &Path, binding_id: &Id) -> PathBuf {
    store
        .join(BINDING_DIRECTORY)
        .join(format!("{}.worker.binding.json", path_segment(binding_id)))
}

fn read_worker_binding_file(path: &Path) -> Result<WorkerBinding, NativeCliError> {
    let binding: WorkerBinding = serde_json::from_value(read_json(path)?)?;
    validate_worker_binding(&binding)
        .map_err(|error| NativeCliError::invalid(format!("{}: {error}", path.display())))?;
    Ok(binding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn binding_storage_round_trips_registered_content() {
        let directory = test_directory();
        let input = directory.join("input.worker.binding.json");
        fs::write(
            &input,
            include_str!("../../../schemas/casegraphen/worker.binding.example.json"),
        )
        .expect("write binding input");

        let result = binding_register(&directory, &input).expect("register binding");
        let binding_id =
            Id::new("worker_binding:native-contract-review").expect("worker binding id");
        let stored =
            read_registered_worker_binding(&directory, &binding_id).expect("read stored binding");

        assert_eq!(result["result"]["binding"], json!(stored));
        assert_eq!(
            result["result"]["binding_content_hash"],
            json!(worker_binding_content_hash(&stored).expect("binding hash"))
        );
        fs::remove_dir_all(directory).expect("remove binding test directory");
    }

    fn test_directory() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let counter = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("casegraphen-binding-test-{nanos}-{counter}"));
        fs::create_dir_all(&path).expect("create binding test directory");
        path
    }
}

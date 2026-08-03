#!/usr/bin/env python3
"""Fail-closed inventory gate for experimental Graph Engineering contracts.

This intentionally governs v0 identity and internal consistency, not backwards
compatibility. Promotion into schemas/casegraphen remains a separate decision.
"""

from __future__ import annotations

import argparse
import copy
import json
import pathlib
import re
import shutil
import tempfile
from typing import Any

import jsonschema
from referencing import Registry, Resource


INVENTORY = pathlib.Path("schemas/experimental/contracts.v0.json")
CONST_RE = re.compile(
    r'pub const\s+([A-Z][A-Z0-9_]*SCHEMA[A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]+)"\s*;',
    re.MULTILINE,
)
CONST_DECL_RE = re.compile(
    r"pub const\s+([A-Z][A-Z0-9_]*SCHEMA[A-Z0-9_]*)\s*:\s*&str\s*=\s*(.*?);",
    re.MULTILINE | re.DOTALL,
)
EXPERIMENTAL_PREFIX = "casegraphen.experimental."


def problem(code: str, detail: str) -> tuple[str, str]:
    return code, detail


def json_pointer_exists(document: Any, fragment: str) -> bool:
    if fragment in ("", "/"):
        return True
    if not fragment.startswith("/"):
        return False
    current = document
    for raw in fragment[1:].split("/"):
        token = raw.replace("~1", "/").replace("~0", "~")
        if isinstance(current, dict) and token in current:
            current = current[token]
        elif isinstance(current, list) and token.isdigit() and int(token) < len(current):
            current = current[int(token)]
        else:
            return False
    return True


def walk_refs(value: Any):
    if isinstance(value, dict):
        for key, child in value.items():
            if key == "$ref" and isinstance(child, str):
                yield child
            yield from walk_refs(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk_refs(child)


def source_constants(root: pathlib.Path) -> dict[str, tuple[str, str]]:
    result: dict[str, tuple[str, str]] = {}
    for source in sorted((root / "src").rglob("*.rs")):
        relative = source.relative_to(root).as_posix()
        for name, value in CONST_RE.findall(source.read_text()):
            if not value.startswith(EXPERIMENTAL_PREFIX):
                continue
            owner = f"{relative}::{name}"
            result[owner] = (value, relative)
    return result


def run_checks(root: pathlib.Path) -> list[tuple[str, str]]:
    problems: list[tuple[str, str]] = []
    experimental = root / "schemas/experimental"
    inventory_path = root / INVENTORY
    try:
        inventory = json.loads(inventory_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        return [problem("invalid_inventory", str(error))]

    contracts = inventory.get("contracts")
    if not isinstance(contracts, list):
        return [problem("invalid_inventory", "contracts must be an array")]

    schema_files = sorted(experimental.glob("*.schema.json"))
    schema_by_file: dict[str, Any] = {}
    schema_by_id: dict[str, Any] = {}
    files_by_id: dict[str, list[str]] = {}
    for path in schema_files:
        try:
            schema = json.loads(path.read_text())
        except json.JSONDecodeError as error:
            problems.append(problem("invalid_schema_json", f"{path}: {error}"))
            continue
        schema_id = schema.get("$id")
        if not isinstance(schema_id, str) or not schema_id:
            problems.append(problem("missing_schema_id", str(path)))
            continue
        schema_by_file[path.name] = schema
        schema_by_id.setdefault(schema_id, schema)
        files_by_id.setdefault(schema_id, []).append(path.name)

    for schema_id, files in files_by_id.items():
        if len(files) != 1:
            problems.append(problem("duplicate_schema_id", f"{schema_id}: {files}"))

    entries_by_id: dict[str, Any] = {}
    entries_by_file: dict[str, Any] = {}
    owned_examples: set[str] = set()
    owners: dict[str, list[str]] = {}
    for index, entry in enumerate(contracts):
        if not isinstance(entry, dict):
            problems.append(problem("invalid_inventory", f"contracts[{index}] is not an object"))
            continue
        contract_id = entry.get("id")
        schema_file = entry.get("schema_file")
        owner = entry.get("rust_owner")
        if not all(isinstance(value, str) and value for value in (contract_id, schema_file, owner)):
            problems.append(problem("invalid_inventory", f"contracts[{index}] lacks id/schema_file/rust_owner"))
            continue
        if contract_id in entries_by_id:
            problems.append(problem("duplicate_inventory_id", contract_id))
        if schema_file in entries_by_file:
            problems.append(problem("duplicate_inventory_file", schema_file))
        entries_by_id[contract_id] = entry
        entries_by_file[schema_file] = entry
        owners.setdefault(owner, []).append(contract_id)
        for example in entry.get("examples", []):
            if example in owned_examples:
                problems.append(problem("duplicate_example_owner", example))
            owned_examples.add(example)

    for schema_file, schema in schema_by_file.items():
        entry = entries_by_file.get(schema_file)
        if entry is None:
            problems.append(problem("orphan_schema", schema_file))
        elif entry["id"] != schema.get("$id"):
            problems.append(problem("inventory_schema_id_mismatch", f"{schema_file}: {entry['id']} != {schema.get('$id')}"))
    for schema_file in entries_by_file:
        if schema_file not in schema_by_file:
            problems.append(problem("missing_schema_file", schema_file))

    for path in sorted(experimental.glob("*.example.json")):
        if path.name not in owned_examples:
            problems.append(problem("orphan_example", path.name))
    for entry in entries_by_id.values():
        examples = entry.get("examples", [])
        exemption = entry.get("report_only_exemption")
        if entry.get("kind") in ("input", "record") and not examples:
            problems.append(problem("missing_required_example", entry["id"]))
        if not examples and not exemption:
            problems.append(problem("unexplained_example_exemption", entry["id"]))
        if exemption and entry.get("kind") != "report" and entry["id"] != "casegraphen.experimental.deployment_bundle.v0":
            problems.append(problem("invalid_report_exemption", entry["id"]))
        for example_name in examples:
            example_path = experimental / example_name
            if not example_path.is_file():
                problems.append(problem("missing_example_file", example_name))

    constants = source_constants(root)
    for source in sorted((root / "src").rglob("*.rs")):
        for name, expression in CONST_DECL_RE.findall(source.read_text()):
            if re.fullmatch(r'"[^"]+"', expression.strip()) is None:
                relative = source.relative_to(root).as_posix()
                problems.append(problem(
                    "nonliteral_schema_constant",
                    f"{relative}::{name} must be a literal so inventory ownership is inspectable",
                ))
    for owner, (value, _) in constants.items():
        entries = owners.get(owner, [])
        if len(entries) != 1:
            problems.append(problem("orphan_rust_constant", f"{owner} ({value}) has {len(entries)} inventory owners"))
        elif entries[0] != value:
            problems.append(problem("rust_constant_id_mismatch", f"{owner}: {value} != {entries[0]}"))
    for owner, ids in owners.items():
        if len(ids) != 1:
            problems.append(problem("duplicate_rust_owner", f"{owner}: {ids}"))
        elif owner not in constants:
            problems.append(problem("unknown_rust_owner", owner))

    for schema_id, schema in schema_by_id.items():
        properties = schema.get("properties", {})
        declared_schema = properties.get("schema", {}).get("const") if isinstance(properties, dict) else None
        if declared_schema is not None and declared_schema != schema_id:
            problems.append(problem("schema_identity_const_mismatch", f"{schema_id}: {declared_schema}"))
        version_match = re.search(r"\.v(\d+)$", schema_id)
        version_const = properties.get("schema_version", {}).get("const") if isinstance(properties, dict) else None
        if version_const is not None and version_match and version_const != int(version_match.group(1)):
            problems.append(problem("stale_schema_version", f"{schema_id}: {version_const}"))
        for reference in walk_refs(schema):
            target_id, separator, fragment = reference.partition("#")
            target = schema if not target_id else schema_by_id.get(target_id) or schema_by_file.get(target_id)
            if target is None:
                problems.append(problem("unknown_reference", f"{schema_id}: {reference}"))
            elif separator and not json_pointer_exists(target, fragment):
                problems.append(problem("unknown_reference_fragment", f"{schema_id}: {reference}"))

    for entry in entries_by_id.values():
        for reference in entry.get("references", []):
            if reference not in entries_by_id:
                problems.append(problem("unknown_contract_reference", f"{entry['id']}: {reference}"))

    if not any(code.startswith("duplicate_schema_id") for code, _ in problems):
        try:
            resources = [
                (schema_id, Resource.from_contents(schema))
                for schema_id, schema in schema_by_id.items()
            ]
            resources.extend(
                (schema_file, Resource.from_contents(schema))
                for schema_file, schema in schema_by_file.items()
            )
            registry = Registry().with_resources(resources)
            for schema_id, schema in schema_by_id.items():
                validator_type = jsonschema.validators.validator_for(schema)
                validator_type.check_schema(schema)
                validator = validator_type(schema, registry=registry)
                entry = entries_by_id.get(schema_id)
                if entry is None:
                    continue
                for example_name in entry.get("examples", []):
                    example_path = experimental / example_name
                    if not example_path.is_file():
                        continue
                    instance = json.loads(example_path.read_text())
                    errors = sorted(validator.iter_errors(instance), key=lambda error: list(error.path))
                    for error in errors:
                        location = ".".join(str(part) for part in error.path) or "$"
                        problems.append(problem("example_schema_mismatch", f"{example_name}:{location}: {error.message}"))
        except Exception as error:  # jsonschema/referencing diagnostics are part of this gate.
            problems.append(problem("schema_registry_failure", str(error)))

    return problems


def mutate_fixture(root: pathlib.Path, mutation: str) -> None:
    experimental = root / "schemas/experimental"
    if mutation == "duplicate_id":
        path = experimental / "control_plane.notification.v0.schema.json"
        value = json.loads(path.read_text())
        value["$id"] = "casegraphen.experimental.control_plane.catalog.v0"
        path.write_text(json.dumps(value))
    elif mutation == "orphan_schema":
        (experimental / "orphan.v0.schema.json").write_text(json.dumps({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "casegraphen.experimental.orphan.v0", "type": "object"
        }))
    elif mutation == "orphan_example":
        (experimental / "orphan.v0.example.json").write_text("{}")
    elif mutation == "stale_version":
        path = experimental / "topology.patch.v0.schema.json"
        value = json.loads(path.read_text())
        value["properties"]["schema_version"]["const"] = 1
        path.write_text(json.dumps(value))
    elif mutation == "unknown_reference":
        path = experimental / "topology.patch.v0.schema.json"
        value = json.loads(path.read_text())
        value.setdefault("$defs", {})["negative_unknown"] = {"$ref": "casegraphen.experimental.missing.v0"}
        path.write_text(json.dumps(value))
    elif mutation == "nonliteral_constant":
        path = root / "src/control_plane.rs"
        path.write_text(path.read_text() + '\npub const NEGATIVE_SCHEMA: &str = concat!("casegraphen.experimental.", "negative.v0");\n')
    else:
        raise ValueError(f"unknown negative fixture mutation: {mutation}")


def run_negative_fixtures(root: pathlib.Path) -> list[tuple[str, str]]:
    failures: list[tuple[str, str]] = []
    fixture_dir = root / "tests/fixtures/experimental-schema-conformance"
    for fixture_path in sorted(fixture_dir.glob("*.json")):
        fixture = json.loads(fixture_path.read_text())
        with tempfile.TemporaryDirectory(prefix="casegraphen-schema-negative-") as temporary:
            copy_root = pathlib.Path(temporary)
            shutil.copytree(root / "schemas", copy_root / "schemas")
            shutil.copytree(root / "src", copy_root / "src")
            mutate_fixture(copy_root, fixture["mutation"])
            codes = {code for code, _ in run_checks(copy_root)}
        if fixture["expected_code"] not in codes:
            failures.append(problem(
                "negative_fixture_did_not_fail",
                f"{fixture_path.name}: expected {fixture['expected_code']}, got {sorted(codes)}",
            ))
    return failures


def validate_instance_bundle(root: pathlib.Path, path: pathlib.Path) -> list[tuple[str, str]]:
    experimental = root / "schemas/experimental"
    schemas = [json.loads(schema_path.read_text()) for schema_path in experimental.glob("*.schema.json")]
    schema_by_id = {schema["$id"]: schema for schema in schemas}
    resources = [
        (schema_id, Resource.from_contents(schema)) for schema_id, schema in schema_by_id.items()
    ]
    resources.extend(
        (schema_path.name, Resource.from_contents(json.loads(schema_path.read_text())))
        for schema_path in experimental.glob("*.schema.json")
    )
    registry = Registry().with_resources(resources)
    failures: list[tuple[str, str]] = []
    for index, item in enumerate(json.loads(path.read_text())):
        schema_id = item.get("schema_id")
        schema = schema_by_id.get(schema_id)
        if schema is None:
            failures.append(problem("unknown_instance_schema", f"instances[{index}]: {schema_id}"))
            continue
        validator_type = jsonschema.validators.validator_for(schema)
        validator = validator_type(schema, registry=registry)
        for error in validator.iter_errors(item.get("instance")):
            location = ".".join(str(part) for part in error.path) or "$"
            failures.append(problem("rust_serialization_schema_mismatch", f"{schema_id}:{location}: {error.message}"))
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="validate the checked-out contracts")
    parser.add_argument("--self-test", action="store_true", help="prove known-bad fixtures fail closed")
    parser.add_argument("--instances", type=pathlib.Path, help="validate Rust-serialized instance bundle")
    arguments = parser.parse_args()
    if not arguments.check and not arguments.self_test and arguments.instances is None:
        parser.error("select --check, --self-test, and/or --instances")
    root = pathlib.Path(__file__).resolve().parent.parent
    problems: list[tuple[str, str]] = []
    if arguments.check:
        problems.extend(run_checks(root))
    if arguments.self_test:
        problems.extend(run_negative_fixtures(root))
    if arguments.instances is not None:
        problems.extend(validate_instance_bundle(root, arguments.instances))
    if problems:
        for code, detail in problems:
            print(f"FAIL [{code}] {detail}")
        return 1
    contract_count = len(json.loads((root / INVENTORY).read_text())["contracts"])
    print(f"ok: {contract_count} governed experimental contracts; negative fixtures fail closed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

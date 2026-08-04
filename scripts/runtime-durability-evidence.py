#!/usr/bin/env python3
"""Build and independently verify retained runtime-durability evidence.

Archive construction is delegated to ``fresh-agent-run-provenance.py`` so the
repository has one deterministic tar/gzip rule. This module owns only the
runtime-durability evidence contract and its offline verification.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import tarfile
import tempfile
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[1]
SHARED_PROVENANCE = ROOT / "scripts/fresh-agent-run-provenance.py"
WORKFLOW_PATH = ".github/workflows/runtime-durability-evidence.yml"
PACKAGE_SCHEMA = "casegraphen.experimental.runtime_durability.release_package.v1"
RETENTION_SCHEMA = "casegraphen.experimental.runtime_durability.retention_record.v1"
SOURCE_SCHEMA = "casegraphen.experimental.runtime_durability_pilot.evidence_manifest.v1"
MAX_PACKAGE_BYTES = 64 * 1024 * 1024
MAX_UNPACKED_BYTES = 256 * 1024 * 1024
MAX_MEMBERS = 64
REQUIRED_ROLES = {
    "aggregate_report",
    "promotion_report",
    "binary_artifact",
    "runtime_pilot_report",
    "execution_topology",
    "runtime_completeness",
    "runtime_expectation",
    "runtime_node_reports",
    "allocator_report",
    "reviewed_resource_report",
    "remote_journal",
}


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def reject_nonfinite(value: str) -> None:
    raise ValueError(f"non-finite JSON number: {value}")


def load_object(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(
            path.read_text(encoding="utf-8"),
            object_pairs_hook=reject_duplicate_keys,
            parse_constant=reject_nonfinite,
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"invalid JSON object: {path}: {error}") from error
    if not isinstance(value, dict):
        raise SystemExit(f"expected JSON object: {path}")
    return value


def canonical(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode()


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def exact_sha(value: str) -> str:
    if re.fullmatch(r"[0-9a-f]{40}", value) is None:
        raise argparse.ArgumentTypeError("expected exact lowercase 40-character SHA")
    return value


def positive_integer(value: str) -> int:
    parsed = int(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("expected positive integer")
    return parsed


def sha256_value(value: Any, label: str) -> str:
    if not isinstance(value, str) or re.fullmatch(r"sha256:[0-9a-f]{64}", value) is None:
        raise SystemExit(f"{label} must be a SHA-256 digest")
    return value


def relative_file(root: pathlib.Path, value: Any) -> pathlib.Path:
    if not isinstance(value, str):
        raise SystemExit("evidence path must be a string")
    relative = pathlib.PurePosixPath(value)
    if relative.is_absolute() or not relative.parts or ".." in relative.parts:
        raise SystemExit(f"unsafe evidence path: {value}")
    path = root.joinpath(*relative.parts)
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"evidence must be a regular non-symlink file: {value}")
    return path


def source_inventory(
    evidence_dir: pathlib.Path,
    manifest_path: pathlib.Path | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    source_path = manifest_path or evidence_dir / "retained-evidence.manifest.json"
    source = load_object(source_path)
    if set(source) != {"schema", "schema_version", "accepted", "files"}:
        raise SystemExit("runtime evidence manifest has missing or unknown fields")
    if source.get("schema") != SOURCE_SCHEMA or source.get("schema_version") != 1:
        raise SystemExit("runtime evidence manifest schema mismatch")
    if source.get("accepted") is not False or not isinstance(source.get("files"), list):
        raise SystemExit("runtime evidence manifest must remain unaccepted")
    files: list[dict[str, Any]] = []
    seen_paths: set[str] = set()
    seen_roles: set[str] = set()
    for item in source["files"]:
        if not isinstance(item, dict) or set(item) != {
            "path", "role", "content_hash", "byte_length"
        }:
            raise SystemExit("invalid runtime evidence inventory entry")
        path = relative_file(evidence_dir, item["path"])
        role = item["role"]
        if not isinstance(role, str) or role not in REQUIRED_ROLES:
            raise SystemExit(f"unknown runtime evidence role: {role!r}")
        if item["path"] in seen_paths or role in seen_roles:
            raise SystemExit("duplicate runtime evidence path or role")
        seen_paths.add(item["path"])
        seen_roles.add(role)
        data = path.read_bytes()
        if item["content_hash"] != digest(data) or item["byte_length"] != len(data):
            raise SystemExit(f"runtime evidence inventory mismatch: {item['path']}")
        files.append(dict(item))
    if seen_roles != REQUIRED_ROLES:
        missing = sorted(REQUIRED_ROLES - seen_roles)
        raise SystemExit(f"runtime evidence roles incomplete: {missing}")
    return source, sorted(files, key=lambda item: item["path"])


def validate_reports(evidence_dir: pathlib.Path, files: list[dict[str, Any]]) -> dict[str, Any]:
    by_role = {item["role"]: item for item in files}
    report = load_object(evidence_dir / by_role["aggregate_report"]["path"])
    promotion = load_object(evidence_dir / by_role["promotion_report"]["path"])
    required_report = {
        "source_revision", "source_worktree_dirty", "accepted", "promotion_eligible",
        "all_thresholds_passed", "topology_content_hash", "reviewed_deployment_hash",
        "harness_content_hash", "contract_content_hashes", "runtime_versions", "reports",
    }
    if not required_report <= set(report):
        raise SystemExit("durability report omits package authority fields")
    if (
        report["source_worktree_dirty"] is not False
        or report["accepted"] is not False
        or report["promotion_eligible"] is not False
        or report["all_thresholds_passed"] is not True
        or promotion.get("accepted") is not False
        or promotion.get("promotion_recommended") is not False
        or promotion.get("durability_thresholds_passed") is not True
    ):
        raise SystemExit("runtime durability evidence is dirty, failed, or claims acceptance")
    sha256_value("sha256:" + str(report["topology_content_hash"]).removeprefix("sha256:"), "topology hash")
    sha256_value("sha256:" + str(report["reviewed_deployment_hash"]).removeprefix("sha256:"), "deployment hash")
    if not isinstance(report["contract_content_hashes"], dict) or not report["contract_content_hashes"]:
        raise SystemExit("runtime durability contract hash inventory is empty")
    for name, value in report["contract_content_hashes"].items():
        if not isinstance(name, str) or re.fullmatch(r"[0-9a-f]{64}", str(value)) is None:
            raise SystemExit("runtime durability contract hash is invalid")
    if not isinstance(report["runtime_versions"], dict) or not report["runtime_versions"]:
        raise SystemExit("runtime durability tool version inventory is empty")
    return report


def shared_package(input_dir: pathlib.Path, output: pathlib.Path) -> None:
    process = subprocess.run(
        [
            "python3", str(SHARED_PROVENANCE), "package", "--input", str(input_dir),
            "--output", str(output),
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if process.returncode != 0:
        raise SystemExit(process.stderr or process.stdout or "shared package command failed")


def build(args: argparse.Namespace) -> int:
    evidence_dir = args.evidence_dir.resolve()
    source, files = source_inventory(evidence_dir)
    report = validate_reports(evidence_dir, files)
    if report["source_revision"] != args.evaluated_commit:
        raise SystemExit("evaluated commit does not match durability report source revision")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    package_path = args.output_dir / "runtime-durability-evidence.tar.gz"
    retention_path = args.output_dir / "retention-record.json"
    if package_path.exists() or retention_path.exists():
        raise SystemExit("runtime durability package output already exists")
    with tempfile.TemporaryDirectory(prefix="casegraphen-runtime-evidence-") as directory:
        staging = pathlib.Path(directory) / "package"
        evidence = staging / "evidence"
        evidence.mkdir(parents=True)
        for item in files:
            shutil.copyfile(evidence_dir / item["path"], evidence / item["path"])
        # The source producer may emit its strict inventory in any order.  The
        # package contract is canonical, so bind the same path-sorted inventory
        # in both manifests before constructing deterministic archive bytes.
        canonical_source = dict(source)
        canonical_source["files"] = files
        source_bytes = canonical(canonical_source) + b"\n"
        (staging / "source-evidence.manifest.json").write_bytes(source_bytes)
        package_manifest = {
            "schema": PACKAGE_SCHEMA,
            "schema_version": 1,
            "accepted": False,
            "promotion_recommended": False,
            "provenance": {
                "repository": args.repository,
                "workflow_path": WORKFLOW_PATH,
                "workflow_run_id": args.workflow_run_id,
                "workflow_run_attempt": args.workflow_run_attempt,
                "evaluated_commit_sha": args.evaluated_commit,
            },
            "evidence": {
                "topology_content_hash": "sha256:" + str(report["topology_content_hash"]).removeprefix("sha256:"),
                "reviewed_deployment_hash": "sha256:" + str(report["reviewed_deployment_hash"]).removeprefix("sha256:"),
                "all_thresholds_passed": True,
                "contract_content_hashes": report["contract_content_hashes"],
                "runtime_versions": report["runtime_versions"],
                "threshold_observations": report["reports"],
                "source_manifest_content_hash": digest(source_bytes),
            },
            "files": files,
        }
        (staging / "runtime-durability-package-manifest.json").write_bytes(
            canonical(package_manifest) + b"\n"
        )
        shared_package(staging, package_path)
    package_bytes = package_path.read_bytes()
    if len(package_bytes) > MAX_PACKAGE_BYTES:
        raise SystemExit("runtime durability package exceeds retention budget")
    package_hash = digest(package_bytes)
    bare_hash = package_hash.removeprefix("sha256:")
    tag = f"runtime-durability-evidence-{bare_hash}"
    asset = f"sha256-{bare_hash}.tar.gz"
    retention = {
        "schema": RETENTION_SCHEMA,
        "schema_version": 1,
        "retention_state": "publication_pending",
        "accepted": False,
        "promotion_recommended": False,
        "release": {
            "repository": args.repository,
            "url": f"https://github.com/{args.repository}/releases/tag/{tag}",
            "tag": tag,
            "asset_name": asset,
            "package_sha256": package_hash,
            "byte_length": len(package_bytes),
        },
        "provenance": package_manifest["provenance"],
        "evidence": {
            "topology_content_hash": package_manifest["evidence"]["topology_content_hash"],
            "reviewed_deployment_hash": package_manifest["evidence"]["reviewed_deployment_hash"],
            "all_thresholds_passed": True,
        },
    }
    retention_path.write_bytes(canonical(retention) + b"\n")
    print(json.dumps({"tag": tag, "asset_name": asset, "package_sha256": package_hash}, sort_keys=True))
    return 0


def safe_extract(asset: pathlib.Path, destination: pathlib.Path) -> None:
    if asset.stat().st_size > MAX_PACKAGE_BYTES:
        raise SystemExit("runtime durability package exceeds compressed-size budget")
    total = 0
    seen: set[str] = set()
    try:
        with tarfile.open(asset, mode="r:gz") as archive:
            members = archive.getmembers()
            if len(members) > MAX_MEMBERS:
                raise SystemExit("runtime durability package has too many members")
            for member in members:
                relative = pathlib.PurePosixPath(member.name)
                if relative.is_absolute() or ".." in relative.parts or member.name in seen:
                    raise SystemExit("runtime durability package has unsafe or duplicate paths")
                seen.add(member.name)
                if not (member.isfile() or member.isdir()):
                    raise SystemExit("runtime durability package contains non-regular content")
                total += member.size
                if total > MAX_UNPACKED_BYTES:
                    raise SystemExit("runtime durability package exceeds unpacked-size budget")
            archive.extractall(destination, filter="data")
    except (tarfile.TarError, OSError) as error:
        raise SystemExit(f"invalid runtime durability package: {error}") from error


def validate_retention(value: dict[str, Any]) -> None:
    if set(value) != {
        "schema", "schema_version", "retention_state", "accepted",
        "promotion_recommended", "release", "provenance", "evidence",
    }:
        raise SystemExit("retention record has missing or unknown fields")
    if (
        value.get("schema") != RETENTION_SCHEMA
        or value.get("schema_version") != 1
        or value.get("retention_state") not in {"publication_pending", "retained_release"}
        or value.get("accepted") is not False
        or value.get("promotion_recommended") is not False
    ):
        raise SystemExit("retention record authority boundary is invalid")
    release = value.get("release")
    provenance = value.get("provenance")
    evidence = value.get("evidence")
    if not all(isinstance(item, dict) for item in (release, provenance, evidence)):
        raise SystemExit("retention record sections must be objects")
    if set(release) != {"repository", "url", "tag", "asset_name", "package_sha256", "byte_length"}:
        raise SystemExit("retention release identity is invalid")
    package_hash = sha256_value(release["package_sha256"], "retained package hash")
    bare = package_hash.removeprefix("sha256:")
    if release["tag"] != f"runtime-durability-evidence-{bare}" or release["asset_name"] != f"sha256-{bare}.tar.gz":
        raise SystemExit("retention tag or asset is not content-addressed")
    if release["url"] != f"https://github.com/{release['repository']}/releases/tag/{release['tag']}":
        raise SystemExit("retention Release URL does not match repository and tag")
    if not isinstance(release["byte_length"], int) or isinstance(release["byte_length"], bool) or release["byte_length"] <= 0:
        raise SystemExit("retention package byte length is invalid")
    if set(provenance) != {"repository", "workflow_path", "workflow_run_id", "workflow_run_attempt", "evaluated_commit_sha"}:
        raise SystemExit("retention workflow provenance is invalid")
    if provenance["repository"] != release["repository"] or provenance["workflow_path"] != WORKFLOW_PATH:
        raise SystemExit("retention workflow repository or path mismatch")
    if re.fullmatch(r"[0-9a-f]{40}", str(provenance["evaluated_commit_sha"])) is None:
        raise SystemExit("retention evaluated commit must be an exact lowercase SHA")
    for field in ("workflow_run_id", "workflow_run_attempt"):
        if not isinstance(provenance[field], int) or isinstance(provenance[field], bool) or provenance[field] <= 0:
            raise SystemExit("retention workflow run identity is invalid")
    if set(evidence) != {"topology_content_hash", "reviewed_deployment_hash", "all_thresholds_passed"}:
        raise SystemExit("retention evidence binding is invalid")
    sha256_value(evidence["topology_content_hash"], "retained topology hash")
    sha256_value(evidence["reviewed_deployment_hash"], "retained deployment hash")
    if evidence["all_thresholds_passed"] is not True:
        raise SystemExit("retained durability evidence did not pass its thresholds")


def verify(args: argparse.Namespace) -> int:
    retention = load_object(args.manifest)
    validate_retention(retention)
    asset_bytes = args.asset.read_bytes()
    if digest(asset_bytes) != retention["release"]["package_sha256"] or len(asset_bytes) != retention["release"]["byte_length"]:
        raise SystemExit("retained package bytes do not match retention record")
    with tempfile.TemporaryDirectory(prefix="casegraphen-runtime-verify-") as directory:
        extracted = pathlib.Path(directory) / "extracted"
        extracted.mkdir()
        safe_extract(args.asset, extracted)
        internal = load_object(extracted / "runtime-durability-package-manifest.json")
        if set(internal) != {
            "schema", "schema_version", "accepted", "promotion_recommended",
            "provenance", "evidence", "files",
        } or internal.get("schema") != PACKAGE_SCHEMA or internal.get("schema_version") != 1:
            raise SystemExit("runtime durability internal package manifest is invalid")
        if internal.get("accepted") is not False or internal.get("promotion_recommended") is not False:
            raise SystemExit("runtime durability package claims authority")
        if internal.get("provenance") != retention["provenance"]:
            raise SystemExit("runtime durability workflow provenance mismatch")
        expected_evidence = retention["evidence"]
        for field in ("topology_content_hash", "reviewed_deployment_hash", "all_thresholds_passed"):
            if internal.get("evidence", {}).get(field) != expected_evidence[field]:
                raise SystemExit(f"runtime durability retained evidence mismatch: {field}")
        source_path = extracted / "source-evidence.manifest.json"
        source_bytes = source_path.read_bytes()
        if internal.get("evidence", {}).get("source_manifest_content_hash") != digest(source_bytes):
            raise SystemExit("runtime durability source manifest hash mismatch")
        source = load_object(source_path)
        files = internal.get("files")
        if not isinstance(files, list) or source.get("files") != files:
            raise SystemExit("runtime durability source/package inventory mismatch")
        expected_members = {
            "runtime-durability-package-manifest.json",
            "source-evidence.manifest.json",
        }
        expected_members.update(
            f"evidence/{item['path']}"
            for item in files
            if isinstance(item, dict) and isinstance(item.get("path"), str)
        )
        actual_members = {
            path.relative_to(extracted).as_posix()
            for path in extracted.rglob("*")
            if path.is_file()
        }
        if actual_members != expected_members:
            raise SystemExit("runtime durability package has missing or unaccounted members")
        temp_evidence = extracted / "evidence"
        _, verified_files = source_inventory(temp_evidence, source_path)
        if verified_files != files:
            raise SystemExit("runtime durability package inventory is not canonical")
        report = validate_reports(temp_evidence, verified_files)
        if report["source_revision"] != retention["provenance"]["evaluated_commit_sha"]:
            raise SystemExit("runtime durability report commit mismatch")
        for field in ("contract_content_hashes", "runtime_versions"):
            if internal["evidence"].get(field) != report[field]:
                raise SystemExit(f"runtime durability internal report mismatch: {field}")
        if internal["evidence"].get("threshold_observations") != report["reports"]:
            raise SystemExit("runtime durability threshold observations mismatch")
        rebuilt = pathlib.Path(directory) / "rebuilt.tar.gz"
        shared_package(extracted, rebuilt)
        if rebuilt.read_bytes() != asset_bytes:
            raise SystemExit("runtime durability archive is not the deterministic canonical package")
    print(json.dumps({"verified": True, "package_sha256": digest(asset_bytes)}, sort_keys=True))
    return 0


def mark_retained(args: argparse.Namespace) -> int:
    value = load_object(args.manifest)
    validate_retention(value)
    if value["retention_state"] != "publication_pending":
        raise SystemExit("only a publication-pending record can become retained")
    if args.output.exists():
        raise SystemExit("retained record output already exists")
    value["retention_state"] = "retained_release"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(canonical(value) + b"\n")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build_parser = commands.add_parser("build-package")
    build_parser.add_argument("--evidence-dir", type=pathlib.Path, required=True)
    build_parser.add_argument("--repository", required=True)
    build_parser.add_argument("--evaluated-commit", type=exact_sha, required=True)
    build_parser.add_argument("--workflow-run-id", type=positive_integer, required=True)
    build_parser.add_argument("--workflow-run-attempt", type=positive_integer, required=True)
    build_parser.add_argument("--output-dir", type=pathlib.Path, required=True)
    build_parser.set_defaults(function=build)
    verify_parser = commands.add_parser("verify-offline")
    verify_parser.add_argument("--manifest", type=pathlib.Path, required=True)
    verify_parser.add_argument("--asset", type=pathlib.Path, required=True)
    verify_parser.set_defaults(function=verify)
    retained_parser = commands.add_parser("mark-retained")
    retained_parser.add_argument("--manifest", type=pathlib.Path, required=True)
    retained_parser.add_argument("--output", type=pathlib.Path, required=True)
    retained_parser.set_defaults(function=mark_retained)
    args = parser.parse_args()
    return args.function(args)


if __name__ == "__main__":
    raise SystemExit(main())

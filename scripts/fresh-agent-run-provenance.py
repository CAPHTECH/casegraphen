#!/usr/bin/env python3
"""Observe GitHub run artifacts and build release evidence without trusting them.

This helper is release-verifier code. Broker and finalizer workflows must run
it from an exact, protected verifier source revision, never from the evaluated
artifact bundle.
"""

from __future__ import annotations

import argparse
import base64
import gzip
import hashlib
import json
import os
import pathlib
import re
import shutil
import stat
import subprocess
import tarfile
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from typing import Any


GITHUB_API_HOST = "api.github.com"
MAX_ARTIFACT_REDIRECTS = 3
GITHUB_ARTIFACT_BLOB_HOST = re.compile(
    r"productionresultssa[0-9]+\.blob\.core\.windows\.net"
)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """Return redirects to the caller so credentials are never auto-forwarded."""

    def redirect_request(self, request, file_pointer, code, message, headers, new_url):
        return None


def canonical(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode()


def sha256(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def reject_nonfinite_constant(value: str) -> None:
    raise ValueError(f"non-finite JSON number: {value}")


def load_object(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(
            path.read_text(),
            object_pairs_hook=reject_duplicate_keys,
            parse_constant=reject_nonfinite_constant,
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"invalid JSON object: {path}: {error}") from error
    if not isinstance(value, dict):
        raise SystemExit(f"{path}: expected JSON object")
    return value


def positive_integer(value: str) -> int:
    parsed = int(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("value must be positive")
    return parsed


def exact_sha(value: str) -> str:
    if not re.fullmatch(r"[0-9a-f]{40}", value):
        raise argparse.ArgumentTypeError("value must be an exact lowercase 40-character SHA")
    return value


def api(
    repository: str,
    path: str,
    token: str,
    opener: urllib.request.OpenerDirector | None = None,
) -> dict[str, Any]:
    if re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository) is None:
        raise SystemExit("GitHub repository must use an exact owner/name slug")
    if not path.startswith("/") or ".." in pathlib.PurePosixPath(path).parts:
        raise SystemExit("GitHub API path is invalid")
    request = urllib.request.Request(
        f"https://api.github.com/repos/{repository}{path}",
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "casegraphen-fresh-agent-verifier",
        },
    )
    try:
        client = opener or urllib.request.build_opener(NoRedirect())
        with client.open(request, timeout=30) as response:
            value = json.loads(
                response.read(),
                object_pairs_hook=reject_duplicate_keys,
                parse_constant=reject_nonfinite_constant,
            )
    except urllib.error.HTTPError as error:
        if error.code in {301, 302, 303, 307, 308}:
            raise SystemExit(f"GitHub API redirect refused: {path}") from error
        raise SystemExit(f"GitHub API request failed: {path}: {error}") from error
    except (urllib.error.URLError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"GitHub API request failed: {path}: {error}") from error
    if not isinstance(value, dict):
        raise SystemExit(f"GitHub API returned a non-object: {path}")
    return value


def artifact_download_host(url: str, crossed_origin: bool) -> tuple[str, bool]:
    parsed = urllib.parse.urlsplit(url)
    try:
        port = parsed.port
    except ValueError as error:
        raise SystemExit("GitHub artifact URL has an invalid port") from error
    if (
        parsed.scheme != "https"
        or parsed.username is not None
        or parsed.password is not None
        or port not in (None, 443)
        or not parsed.hostname
    ):
        raise SystemExit("GitHub artifact URL must be credential-free HTTPS on port 443")
    host = parsed.hostname.casefold()
    if host == GITHUB_API_HOST:
        if crossed_origin:
            raise SystemExit("GitHub artifact redirect cannot return to the authenticated API origin")
        return host, False
    if GITHUB_ARTIFACT_BLOB_HOST.fullmatch(host) is None:
        raise SystemExit(f"GitHub artifact redirect host is not allowlisted: {host}")
    return host, True


def download(
    url: str,
    token: str,
    opener: urllib.request.OpenerDirector | None = None,
) -> bytes:
    """Download an Actions artifact without forwarding the token to blob storage."""

    client = opener or urllib.request.build_opener(NoRedirect())
    current_url = url
    crossed_origin = False
    for redirect_count in range(MAX_ARTIFACT_REDIRECTS + 1):
        host, now_crossed = artifact_download_host(current_url, crossed_origin)
        crossed_origin = crossed_origin or now_crossed
        headers = {"User-Agent": "casegraphen-fresh-agent-verifier"}
        if host == GITHUB_API_HOST:
            headers.update(
                {
                    "Accept": "application/vnd.github+json",
                    "Authorization": f"Bearer {token}",
                    "X-GitHub-Api-Version": "2022-11-28",
                }
            )
        request = urllib.request.Request(current_url, headers=headers)
        try:
            with client.open(request, timeout=120) as response:
                return response.read()
        except urllib.error.HTTPError as error:
            if error.code not in {301, 302, 303, 307, 308}:
                raise SystemExit(f"GitHub artifact download failed: {error}") from error
            location = error.headers.get("Location")
            if not location:
                raise SystemExit("GitHub artifact redirect omitted Location") from error
            if redirect_count == MAX_ARTIFACT_REDIRECTS:
                raise SystemExit("GitHub artifact download exceeded redirect limit") from error
            current_url = urllib.parse.urljoin(current_url, location)
        except urllib.error.URLError as error:
            raise SystemExit(f"GitHub artifact download failed: {error}") from error
    raise SystemExit("GitHub artifact download exceeded redirect limit")


def materialize_zip(data: bytes, destination: pathlib.Path) -> None:
    if destination.exists():
        raise SystemExit(f"artifact destination already exists: {destination}")
    destination.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="casegraphen-artifact-") as directory:
        archive = pathlib.Path(directory) / "artifact.zip"
        archive.write_bytes(data)
        try:
            with zipfile.ZipFile(archive) as zipped:
                for info in zipped.infolist():
                    relative = pathlib.PurePosixPath(info.filename)
                    if relative.is_absolute() or ".." in relative.parts:
                        raise SystemExit(f"unsafe artifact member: {info.filename}")
                    mode = (info.external_attr >> 16) & 0o170000
                    if mode == 0o120000:
                        raise SystemExit(f"symlinked artifact member: {info.filename}")
                    target = destination.joinpath(*relative.parts)
                    if info.is_dir():
                        target.mkdir(parents=True, exist_ok=True)
                        continue
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with zipped.open(info) as source, target.open("xb") as output:
                        shutil.copyfileobj(source, output)
        except zipfile.BadZipFile as error:
            raise SystemExit("GitHub artifact is not a valid ZIP archive") from error


def parse_artifact(value: str) -> tuple[str, pathlib.Path]:
    name, separator, destination = value.partition("=")
    if not separator or not name or not destination:
        raise argparse.ArgumentTypeError("artifact must use name=destination")
    return name, pathlib.Path(destination)


def provider_paths(values: list[str], argument: str) -> dict[str, pathlib.Path]:
    parsed: dict[str, pathlib.Path] = {}
    for value in values:
        provider, separator, path = value.partition("=")
        if separator != "=" or provider not in {"codex", "claude"} or not path:
            raise SystemExit(f"{argument} must use provider=/absolute/or/relative/path")
        if provider in parsed:
            raise SystemExit(f"duplicate {argument} for {provider}")
        parsed[provider] = pathlib.Path(path)
    if set(parsed) != {"codex", "claude"}:
        raise SystemExit(f"{argument} requires exactly codex and claude")
    return parsed


def valid_provider_provenance(value: Any) -> bool:
    required = {
        "evaluated_commit_sha",
        "repository",
        "source_workflow",
        "source_workflow_id",
        "source_workflow_path",
        "source_run_id",
        "source_run_attempt",
        "source_head_ref",
        "source_head_sha",
        "source_event",
        "source_conclusion",
        "provider_artifact",
    }
    if not isinstance(value, dict) or set(value) != required:
        return False
    string_fields = required - {
        "source_workflow_id",
        "source_run_id",
        "source_run_attempt",
        "provider_artifact",
    }
    if not all(isinstance(value[field], str) and value[field] for field in string_fields):
        return False
    if not re.fullmatch(r"[0-9a-f]{40}", value["evaluated_commit_sha"]):
        return False
    if value["source_head_sha"] != value["evaluated_commit_sha"]:
        return False
    if not all(
        isinstance(value[field], int) and not isinstance(value[field], bool) and value[field] > 0
        for field in ("source_workflow_id", "source_run_id", "source_run_attempt")
    ):
        return False
    artifact = value["provider_artifact"]
    return (
        isinstance(artifact, dict)
        and set(artifact) == {"id", "name", "digest"}
        and isinstance(artifact["id"], int)
        and not isinstance(artifact["id"], bool)
        and artifact["id"] > 0
        and isinstance(artifact["name"], str)
        and bool(artifact["name"])
        and isinstance(artifact["digest"], str)
        and re.fullmatch(r"sha256:[0-9a-f]{64}", artifact["digest"]) is not None
    )


def observe(args: argparse.Namespace) -> int:
    token = os.environ.get(args.token_env)
    if not token:
        raise SystemExit(f"missing GitHub token environment: {args.token_env}")
    run = api(args.repository, f"/actions/runs/{args.run_id}", token)
    expected = {
        "id": args.run_id,
        "run_attempt": args.run_attempt,
        "head_sha": args.head_sha,
        "event": args.event,
        "conclusion": args.conclusion,
    }
    for field, value in expected.items():
        if run.get(field) != value:
            raise SystemExit(f"source run mismatch: {field}: {run.get(field)!r} != {value!r}")
    if run.get("head_branch") != args.head_branch:
        raise SystemExit("source run head branch mismatch")
    if run.get("path") != args.workflow_path:
        raise SystemExit(f"source workflow path mismatch: {run.get('path')!r}")
    workflow_id = run.get("workflow_id")
    if not isinstance(workflow_id, int) or workflow_id <= 0:
        raise SystemExit("source workflow id is unavailable")
    artifacts_response = api(
        args.repository, f"/actions/runs/{args.run_id}/artifacts?per_page=100", token
    )
    artifacts = artifacts_response.get("artifacts")
    if not isinstance(artifacts, list):
        raise SystemExit("source artifact inventory is unavailable")

    observations: list[dict[str, Any]] = []
    for name, destination in args.artifact:
        matches = [item for item in artifacts if isinstance(item, dict) and item.get("name") == name]
        if len(matches) != 1:
            raise SystemExit(f"expected exactly one source artifact named {name}, found {len(matches)}")
        item = matches[0]
        digest = item.get("digest")
        if item.get("expired") is not False or not re.fullmatch(r"sha256:[0-9a-f]{64}", digest or ""):
            raise SystemExit(f"source artifact is expired or has no SHA-256 digest: {name}")
        workflow_run = item.get("workflow_run") or {}
        if workflow_run.get("id") != args.run_id or workflow_run.get("head_sha") != args.head_sha:
            raise SystemExit(f"source artifact workflow binding mismatch: {name}")
        archive = download(str(item.get("archive_download_url", "")), token)
        if sha256(archive) != digest:
            raise SystemExit(f"source artifact archive digest mismatch: {name}")
        materialize_zip(archive, destination)
        observations.append(
            {
                "id": item.get("id"),
                "name": name,
                "digest": digest,
            }
        )

    head_ref = f"refs/heads/{args.head_branch}"
    if len(observations) != 1:
        raise SystemExit("exactly one artifact must be observed per provenance document")
    provenance = {
        "evaluated_commit_sha": args.head_sha,
        "repository": args.repository,
        "source_workflow": run.get("name"),
        "source_workflow_id": workflow_id,
        "source_workflow_path": args.workflow_path,
        "source_run_id": args.run_id,
        "source_run_attempt": args.run_attempt,
        "source_head_ref": head_ref,
        "source_head_sha": args.head_sha,
        "source_event": args.event,
        "source_conclusion": args.conclusion,
        "provider_artifact": observations[0],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(provenance, indent=2, sort_keys=True, allow_nan=False) + "\n"
    )
    return 0


def sign_review(args: argparse.Namespace) -> int:
    input_path = args.input
    if input_path.is_symlink():
        raise SystemExit("manual review input must not be a symlink")
    try:
        resolved_input = input_path.resolve(strict=True)
        input_mode = resolved_input.stat().st_mode
    except OSError as error:
        raise SystemExit(f"manual review input is unavailable: {input_path}") from error
    if not stat.S_ISREG(input_mode):
        raise SystemExit("manual review input must be a regular file")
    if args.allowed_review_root is not None:
        allowed_root = args.allowed_review_root
        if allowed_root.is_symlink():
            raise SystemExit("allowed review root must not be a symlink")
        try:
            resolved_root = allowed_root.resolve(strict=True)
        except OSError as error:
            raise SystemExit(f"allowed review root is unavailable: {allowed_root}") from error
        if not resolved_root.is_dir():
            raise SystemExit("allowed review root must be a directory")
        try:
            resolved_input.relative_to(resolved_root)
        except ValueError as error:
            raise SystemExit("manual review input escapes the allowed review root") from error

    value = load_object(resolved_input)
    if not isinstance(value, dict) or "ed25519_signature" in value:
        raise SystemExit("manual review input must be an unsigned JSON object")
    provenance_paths = provider_paths(args.expected_provenance, "--expected-provenance")
    expected_provider_provenance = {
        provider: load_object(path) for provider, path in sorted(provenance_paths.items())
    }
    for provider, provenance in expected_provider_provenance.items():
        if not valid_provider_provenance(provenance):
            raise SystemExit(f"invalid expected provider provenance: {provider}")
    value["schema"] = "casegraphen.eval.fresh_agent_manual_review.v1"
    value["signature_algorithm"] = "ed25519"
    value["reviewer_identity"] = args.reviewer_identity
    value["reviewer_key_id"] = args.reviewer_key_id
    value["expected_provider_provenance"] = expected_provider_provenance
    key_check = subprocess.run(
        ["openssl", "pkey", "-in", str(args.private_key), "-text_pub", "-noout"],
        capture_output=True,
        text=True,
        check=False,
    )
    if key_check.returncode != 0 or not key_check.stdout.startswith("ED25519 Public-Key:\n"):
        raise SystemExit("manual review private key must be an Ed25519 key")
    payload = canonical(value)
    with tempfile.TemporaryDirectory(prefix="casegraphen-review-sign-") as directory:
        payload_path = pathlib.Path(directory) / "payload"
        signature_path = pathlib.Path(directory) / "signature"
        payload_path.write_bytes(payload)
        process = subprocess.run(
            [
                "openssl", "pkeyutl", "-sign", "-inkey", str(args.private_key),
                "-rawin", "-in", str(payload_path), "-out", str(signature_path),
            ],
            capture_output=True,
            check=False,
        )
        if process.returncode != 0:
            raise SystemExit("manual review Ed25519 signing failed")
        value["ed25519_signature"] = base64.b64encode(signature_path.read_bytes()).decode("ascii")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n"
    )
    return 0


def package(args: argparse.Namespace) -> int:
    if args.output.exists():
        raise SystemExit(f"package already exists: {args.output}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("xb") as raw_output:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw_output, compresslevel=9, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
                for path in sorted(args.input.rglob("*")):
                    if path.is_symlink():
                        raise SystemExit(f"refusing symlink in durable package: {path}")
                    if not path.is_file():
                        continue
                    info = archive.gettarinfo(str(path), path.relative_to(args.input).as_posix())
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = 0
                    info.mode = 0o644
                    with path.open("rb") as source:
                        archive.addfile(info, source)
    result = {"sha256": sha256(args.output.read_bytes()), "byte_length": args.output.stat().st_size}
    if args.metadata:
        args.metadata.write_text(
            json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + "\n"
        )
    else:
        print(json.dumps(result, sort_keys=True, allow_nan=False))
    return 0


def verify_file(args: argparse.Namespace) -> int:
    actual = sha256(args.input.read_bytes())
    if actual != args.expected_sha256:
        raise SystemExit(f"file digest mismatch: {actual} != {args.expected_sha256}")
    return 0


def verify_public_key(args: argparse.Namespace) -> int:
    type_check = subprocess.run(
        ["openssl", "pkey", "-pubin", "-in", str(args.input), "-text_pub", "-noout"],
        capture_output=True,
        text=True,
        check=False,
    )
    if type_check.returncode != 0 or not type_check.stdout.startswith("ED25519 Public-Key:\n"):
        raise SystemExit("verification public key must be an Ed25519 public key")
    process = subprocess.run(
        ["openssl", "pkey", "-pubin", "-in", str(args.input), "-outform", "DER"],
        capture_output=True,
        check=False,
    )
    if process.returncode != 0:
        raise SystemExit("invalid public key PEM")
    fingerprint = sha256(process.stdout)
    if fingerprint != args.expected_spki_sha256:
        raise SystemExit(
            f"public key SPKI fingerprint mismatch: {fingerprint} != {args.expected_spki_sha256}"
        )
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(
                {
                    "schema": "casegraphen.eval.verification_key_provenance.v1",
                    "key_id": args.key_id,
                    "algorithm": "ed25519",
                    "spki_sha256": fingerprint,
                },
                indent=2,
                sort_keys=True,
                allow_nan=False,
            )
            + "\n"
        )
    return 0


def record_trusted_source(args: argparse.Namespace) -> int:
    files: list[dict[str, Any]] = []
    for relative in args.file:
        path = args.root / relative
        if not path.is_file() or path.is_symlink():
            raise SystemExit(f"trusted verifier file is unavailable or unsafe: {relative}")
        data = path.read_bytes()
        files.append(
            {
                "path": relative.as_posix(),
                "sha256": sha256(data),
                "byte_length": len(data),
            }
        )
    document = {
        "schema": "casegraphen.eval.trusted_verifier_source.v1",
        "repository": args.repository,
        "source_sha": args.source_sha,
        "files": files,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, indent=2, sort_keys=True, allow_nan=False) + "\n"
    )
    return 0


def inspect_release(args: argparse.Namespace) -> int:
    value = load_object(args.input)
    if not isinstance(value, dict):
        raise SystemExit("release observation must be a JSON object")
    if value.get("tagName") != args.tag or value.get("targetCommitish") != args.target_sha:
        raise SystemExit("durable release tag or target commit mismatch")
    assets = value.get("assets")
    if not isinstance(assets, list):
        raise SystemExit("durable release asset inventory is unavailable")
    if not assets:
        print("absent")
        return 0
    if len(assets) != 1 or not isinstance(assets[0], dict):
        raise SystemExit("durable release contains duplicate or unexpected assets")
    asset = assets[0]
    if asset.get("name") != args.asset_name:
        raise SystemExit("durable release contains an unexpected asset")
    if asset.get("size") != args.expected_size:
        raise SystemExit("durable release asset size mismatch")
    observed_digest = asset.get("digest")
    if observed_digest not in (None, "", args.expected_sha256):
        raise SystemExit("durable release asset digest mismatch")
    print("present")
    return 0


def verify_trusted_source(args: argparse.Namespace) -> int:
    value = load_object(args.input)
    if not isinstance(value, dict) or value.get("schema") != "casegraphen.eval.trusted_verifier_source.v1":
        raise SystemExit("trusted verifier source record is invalid")
    if value.get("repository") != args.repository or value.get("source_sha") != args.source_sha:
        raise SystemExit("trusted verifier source repository or SHA mismatch")
    files = value.get("files")
    if not isinstance(files, list) or not files:
        raise SystemExit("trusted verifier source file inventory is empty")
    seen: set[str] = set()
    for item in files:
        if not isinstance(item, dict) or set(item) != {"path", "sha256", "byte_length"}:
            raise SystemExit("trusted verifier source file entry is invalid")
        relative = pathlib.PurePosixPath(str(item["path"]))
        if relative.is_absolute() or ".." in relative.parts or relative.as_posix() in seen:
            raise SystemExit("trusted verifier source file path is unsafe or duplicated")
        seen.add(relative.as_posix())
        path = args.root.joinpath(*relative.parts)
        if not path.is_file() or path.is_symlink():
            raise SystemExit(f"trusted verifier source file is unavailable: {relative}")
        data = path.read_bytes()
        if item["sha256"] != sha256(data) or item["byte_length"] != len(data):
            raise SystemExit(f"trusted verifier source file mismatch: {relative}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    observe_parser = subparsers.add_parser("observe-run")
    observe_parser.add_argument("--repository", required=True)
    observe_parser.add_argument("--run-id", type=positive_integer, required=True)
    observe_parser.add_argument("--run-attempt", type=positive_integer, required=True)
    observe_parser.add_argument("--head-sha", type=exact_sha, required=True)
    observe_parser.add_argument("--head-branch", default="main")
    observe_parser.add_argument("--workflow-path", required=True)
    observe_parser.add_argument("--event", default="workflow_dispatch")
    observe_parser.add_argument("--conclusion", default="success")
    observe_parser.add_argument("--artifact", type=parse_artifact, action="append", default=[])
    observe_parser.add_argument("--token-env", default="GITHUB_TOKEN")
    observe_parser.add_argument("--output", type=pathlib.Path, required=True)
    observe_parser.set_defaults(function=observe)

    sign_parser = subparsers.add_parser("sign-review")
    sign_parser.add_argument("--input", type=pathlib.Path, required=True)
    sign_parser.add_argument("--output", type=pathlib.Path, required=True)
    sign_parser.add_argument("--private-key", type=pathlib.Path, required=True)
    sign_parser.add_argument("--reviewer-identity", required=True)
    sign_parser.add_argument("--reviewer-key-id", required=True)
    sign_parser.add_argument("--expected-provenance", action="append", required=True)
    sign_parser.add_argument("--allowed-review-root", type=pathlib.Path)
    sign_parser.set_defaults(function=sign_review)

    package_parser = subparsers.add_parser("package")
    package_parser.add_argument("--input", type=pathlib.Path, required=True)
    package_parser.add_argument("--output", type=pathlib.Path, required=True)
    package_parser.add_argument("--metadata", type=pathlib.Path)
    package_parser.set_defaults(function=package)

    verify_parser = subparsers.add_parser("verify-file")
    verify_parser.add_argument("--input", type=pathlib.Path, required=True)
    verify_parser.add_argument("--expected-sha256", required=True)
    verify_parser.set_defaults(function=verify_file)

    key_parser = subparsers.add_parser("verify-public-key")
    key_parser.add_argument("--input", type=pathlib.Path, required=True)
    key_parser.add_argument("--expected-spki-sha256", required=True)
    key_parser.add_argument("--key-id", required=True)
    key_parser.add_argument("--output", type=pathlib.Path)
    key_parser.set_defaults(function=verify_public_key)

    source_parser = subparsers.add_parser("record-trusted-source")
    source_parser.add_argument("--root", type=pathlib.Path, required=True)
    source_parser.add_argument("--repository", required=True)
    source_parser.add_argument("--source-sha", type=exact_sha, required=True)
    source_parser.add_argument("--file", type=pathlib.PurePosixPath, action="append", required=True)
    source_parser.add_argument("--output", type=pathlib.Path, required=True)
    source_parser.set_defaults(function=record_trusted_source)

    release_parser = subparsers.add_parser("inspect-release")
    release_parser.add_argument("--input", type=pathlib.Path, required=True)
    release_parser.add_argument("--tag", required=True)
    release_parser.add_argument("--target-sha", type=exact_sha, required=True)
    release_parser.add_argument("--asset-name", required=True)
    release_parser.add_argument("--expected-size", type=positive_integer, required=True)
    release_parser.add_argument("--expected-sha256", required=True)
    release_parser.set_defaults(function=inspect_release)

    trusted_parser = subparsers.add_parser("verify-trusted-source")
    trusted_parser.add_argument("--input", type=pathlib.Path, required=True)
    trusted_parser.add_argument("--root", type=pathlib.Path, required=True)
    trusted_parser.add_argument("--repository", required=True)
    trusted_parser.add_argument("--source-sha", type=exact_sha, required=True)
    trusted_parser.set_defaults(function=verify_trusted_source)
    args = parser.parse_args()
    return args.function(args)


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Focused regression tests for credential-safe Actions artifact redirects."""

from __future__ import annotations

import importlib.util
import io
import pathlib
import urllib.error


ROOT = pathlib.Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location(
    "fresh_agent_run_provenance", ROOT / "scripts/fresh-agent-run-provenance.py"
)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

API_URL = "https://api.github.com/repos/CAPHTECH/casegraphen/actions/artifacts/1/zip"
BLOB_URL = (
    "https://productionresultssa1.blob.core.windows.net/actions-results/"
    "trusted-artifact.zip?sig=opaque"
)


class Response:
    def __init__(self, data: bytes):
        self.data = data

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        return False

    def read(self) -> bytes:
        return self.data


class RedirectingOpener:
    def __init__(self, locations: list[str], data: bytes = b"artifact"):
        self.locations = list(locations)
        self.data = data
        self.requests = []

    def open(self, request, timeout):
        self.requests.append(request)
        if self.locations:
            headers = {"Location": self.locations.pop(0)}
            raise urllib.error.HTTPError(
                request.full_url, 302, "Found", headers, io.BytesIO()
            )
        return Response(self.data)


def headers(request) -> dict[str, str]:
    return {key.casefold(): value for key, value in request.header_items()}


def expect_refusal(opener: RedirectingOpener) -> None:
    try:
        MODULE.download(API_URL, "repo-token", opener=opener)
    except SystemExit:
        return
    raise AssertionError("unsafe artifact redirect was accepted")


safe = RedirectingOpener([BLOB_URL])
assert MODULE.download(API_URL, "repo-token", opener=safe) == b"artifact"
assert headers(safe.requests[0])["authorization"] == "Bearer repo-token"
assert "authorization" not in headers(safe.requests[1])
assert safe.requests[1].full_url == BLOB_URL

unexpected = RedirectingOpener(["https://attacker.example/artifact.zip"])
expect_refusal(unexpected)
assert len(unexpected.requests) == 1

downgrade = RedirectingOpener(
    ["http://productionresultssa1.blob.core.windows.net/actions-results/artifact.zip"]
)
expect_refusal(downgrade)
assert len(downgrade.requests) == 1

return_to_api = RedirectingOpener(
    [BLOB_URL, "https://api.github.com/repos/CAPHTECH/casegraphen/actions/artifacts/1/zip"]
)
expect_refusal(return_to_api)
assert len(return_to_api.requests) == 2

api_redirect = RedirectingOpener(["https://attacker.example/api-copy"])
try:
    MODULE.api("CAPHTECH/casegraphen", "/actions/runs/1", "repo-token", api_redirect)
except SystemExit as error:
    assert "API redirect refused" in str(error)
else:
    raise AssertionError("authenticated GitHub API redirect was accepted")
assert len(api_redirect.requests) == 1
assert headers(api_redirect.requests[0])["authorization"] == "Bearer repo-token"

print("fresh-agent artifact redirect boundary self-test passed")

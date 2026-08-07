#!/usr/bin/env python3
"""Generates the pilot-corpus-derived adversarial fixtures under
tests/fixtures/github-evidence/ by copying the frozen pilot source bytes
(docs/pilots/issue-102/source/, never modified) and mutating exactly the
one artifact each case needs. Content hashes are recomputed only for the
mutated artifact; every unmutated artifact keeps its manifest content_hash
identical to the pilot's own manifest, byte-for-byte.

Run from anywhere: `python3 tests/fixtures/github-evidence/generate_pilot_derived.py`.
Determinism (design doc §5) means re-running this reproduces the same bytes
committed here; it is not part of the test or build gate.
"""
import hashlib
import json
import os
import shutil

REPO_ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
PILOT_SOURCE = os.path.join(REPO_ROOT, "docs/pilots/issue-102/source")
PILOT_MANIFEST = os.path.join(REPO_ROOT, "docs/pilots/issue-102/capture_manifest.v0.json")
FIXTURES_ROOT = os.path.join(REPO_ROOT, "tests/fixtures/github-evidence")

HEAD_SHA = "c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b"
OLDER_SHA = "5403673f13b45d8deb0f4be62f50390172071bb0"


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_pilot_manifest():
    with open(PILOT_MANIFEST, "rb") as fh:
        return json.load(fh)


def write_json(path: str, value) -> bytes:
    data = json.dumps(value, ensure_ascii=False, indent=2).encode("utf-8") + b"\n"
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as fh:
        fh.write(data)
    return data


def build_pilot_copy_fixture(name: str, mutate_reviews=None, mutate_pr=None,
                              extra_top_level=None):
    """Copies the six pilot source files verbatim into
    tests/fixtures/github-evidence/<name>/source/, then applies
    `mutate_reviews`/`mutate_pr` to the loaded JSON of the reviews / pr
    artifact before rewriting it (recomputing its content_hash only).
    Unmutated artifacts keep the pilot's own manifest content_hash exactly
    (proving the copy is byte-identical to the frozen corpus)."""
    fixture_dir = os.path.join(FIXTURES_ROOT, name)
    if os.path.exists(fixture_dir):
        shutil.rmtree(fixture_dir)
    source_dir = os.path.join(fixture_dir, "source")
    os.makedirs(source_dir)

    manifest = json.loads(json.dumps(load_pilot_manifest()))  # deep copy
    if extra_top_level:
        manifest.update(extra_top_level)

    for entry in manifest["entries"]:
        artifact_path = entry["artifact_path"]
        src = os.path.join(REPO_ROOT, "docs/pilots/issue-102", artifact_path)
        dst = os.path.join(fixture_dir, artifact_path)
        if entry["category"] == "reviews" and mutate_reviews is not None:
            with open(src, "rb") as fh:
                value = json.load(fh)
            mutate_reviews(value)
            data = write_json(dst, value)
            entry["content_hash"] = f"sha256:{sha256_hex(data)}"
        elif entry["category"] in ("pr", "files") and mutate_pr is not None:
            with open(src, "rb") as fh:
                value = json.load(fh)
            mutate_pr(value)
            data = write_json(dst, value)
            entry["content_hash"] = f"sha256:{sha256_hex(data)}"
            # pr and files share the same artifact_path/content_hash in the
            # pilot manifest (one gh pr view --json bundle serves both
            # categories) -- keep that invariant for the mutated copy too.
            for other in manifest["entries"]:
                if other["artifact_path"] == artifact_path:
                    other["content_hash"] = entry["content_hash"]
        else:
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            shutil.copyfile(src, dst)
            # content_hash is already correct (copied verbatim from the pilot).

    manifest_path = os.path.join(fixture_dir, "capture_manifest.v0.json")
    write_json(manifest_path, manifest)
    print(f"built {name} -> {fixture_dir}")


# ---------------------------------------------------------------------
# Team-lead case 1: bot approval at the exact head.
# All 10 coderabbitai reviews flipped to APPROVED and rebound to head.
# ---------------------------------------------------------------------
def mutate_bot_approval_at_head(value):
    pr = value["data"]["repository"]["pullRequest"]
    for node in pr["reviews"]["nodes"]:
        author = node["author"]
        if author.get("__typename") == "Bot" or author.get("login", "").startswith("coderabbitai"):
            node["state"] = "APPROVED"
            node["commit"]["oid"] = HEAD_SHA


build_pilot_copy_fixture("bot-approval-at-head", mutate_reviews=mutate_bot_approval_at_head)


# ---------------------------------------------------------------------
# Team-lead cases 2 & 3: positive control (outside human approval at head
# satisfies) and its pair (same review, older binding -> excluded).
# ---------------------------------------------------------------------
def append_outside_review(commit_sha):
    def mutate(value):
        pr = value["data"]["repository"]["pullRequest"]
        nodes = pr["reviews"]["nodes"]
        nodes.append({
            "id": "PRR_synthetic_outside_reviewer",
            "state": "APPROVED",
            "body": "",
            "submittedAt": "2026-08-06T09:30:00Z",
            "lastEditedAt": None,
            "url": "https://github.com/CAPHTECH/casegraphen/pull/101#pullrequestreview-9999999999",
            "authorAssociation": "NONE",
            "commit": {"oid": commit_sha},
            "author": {
                "__typename": "User",
                "login": "outside-reviewer",
                "id": "MDQ6VXNlcjk5OTk5OQ==",
            },
        })
        pr["reviews"]["totalCount"] = len(nodes)
    return mutate


build_pilot_copy_fixture(
    "positive-control-outside-approval",
    mutate_reviews=append_outside_review(HEAD_SHA),
)
build_pilot_copy_fixture(
    "positive-control-older-binding",
    mutate_reviews=append_outside_review(OLDER_SHA),
)


# ---------------------------------------------------------------------
# Team-lead case 4: caller-declared approval planted inside provider JSON
# (raw reviews artifact). Fields are never read by the allowlist parser.
# ---------------------------------------------------------------------
def mutate_caller_declared_raw_fields(value):
    pr = value["data"]["repository"]["pullRequest"]
    for node in pr["reviews"]["nodes"]:
        node["trusted"] = True
        node["approved"] = True
        node["accepted"] = True
        node["authority"] = "root"
        node["evidence_role"] = "independent_human_candidate"
        node["independence_proven"] = True
        node["review_state"] = "APPROVED"  # bogus: the real field is `state`


build_pilot_copy_fixture(
    "caller-declared-approval-raw-fields",
    mutate_reviews=mutate_caller_declared_raw_fields,
)


# ---------------------------------------------------------------------
# Team-lead case 5: caller-declared trust in the manifest *wrapper* itself
# (`"trusted": true` at the top level). `CaptureManifest` is
# `deny_unknown_fields`, so this must be refused at parse -- before any
# capture-dir byte is read. No mutated capture copy is needed: the test
# points `--capture-dir` at the real pilot corpus directly, since parsing
# never gets that far.
# ---------------------------------------------------------------------
def build_caller_declared_trust_manifest_wrapper_fixture():
    fixture_dir = os.path.join(FIXTURES_ROOT, "caller-declared-trust-manifest-wrapper")
    if os.path.exists(fixture_dir):
        shutil.rmtree(fixture_dir)
    manifest = load_pilot_manifest()
    manifest["trusted"] = True
    write_json(os.path.join(fixture_dir, "manifest.json"), manifest)
    print(f"built caller-declared-trust-manifest-wrapper -> {fixture_dir}")


build_caller_declared_trust_manifest_wrapper_fixture()


# ---------------------------------------------------------------------
# Design row 4: missing actor attestation -> unattributed.
# Built from the retained real unattested `reviews` section of pr-101.json
# (author objects are `{"login": ...}` only -- real provider bytes). The
# GraphQL reviews artifact wire shape requires a `url` per node (which this
# gh --json section does not carry), so a stable url is synthesized from the
# PR url + the review's own retained node id -- the only field added; every
# person-identifying field (author, authorAssociation, state, body, commit)
# is copied verbatim from the retained corpus.
# ---------------------------------------------------------------------
def build_missing_attestation_fixture():
    with open(os.path.join(PILOT_SOURCE, "pr-101.json"), "rb") as fh:
        pr_json = json.load(fh)
    real_reviews = pr_json["reviews"]
    nodes = []
    for review in real_reviews:
        nodes.append({
            "state": review["state"],
            "body": review["body"],
            "submittedAt": review["submittedAt"],
            "lastEditedAt": None,
            "url": f"https://github.com/CAPHTECH/casegraphen/pull/101#{review['id']}",
            "authorAssociation": review["authorAssociation"],
            "commit": {"oid": review["commit"]["oid"]},
            "author": review["author"],  # verbatim: {"login": ...} only, real bytes
        })
    reviews_value = {
        "data": {"repository": {"pullRequest": {
            "number": 101,
            "reviews": {"totalCount": len(nodes), "nodes": nodes},
        }}}
    }
    build_pilot_copy_fixture(
        "missing-actor-attestation",
        mutate_reviews=lambda value: value.update(reviews_value),
    )


build_missing_attestation_fixture()

print("done")

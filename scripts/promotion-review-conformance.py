#!/usr/bin/env python3
"""Reject stale Graph Engineering promotion triggers."""

from __future__ import annotations

import datetime
import hashlib
import json
import pathlib
import re
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[1]
INVENTORY_PATH = ROOT / "docs/reviews/graph-engineering-v0-promotion.inventory.json"
REVIEW_PATH = ROOT / "docs/reviews/graph-engineering-v0-promotion-2026-08-03.md"
# owner_issue values point at GitHub issues, but this gate must not need `gh`
# auth or network access to run. Instead it checks owner_issue state against
# this hand-refreshed, in-repo ledger. That does not remove the drift risk,
# it relocates it: an issue can still close on GitHub without anyone updating
# the ledger. ISSUE_STATE_MAX_AGE_DAYS bounds how long that can go unnoticed —
# past that age the gate fails closed and demands a refresh via `gh issue
# view`, rather than silently trusting old "open" state forever.
ISSUE_STATE_PATH = ROOT / "docs/reviews/promotion-blocker-issue-state.v0.json"
ISSUE_STATE_MAX_AGE_DAYS = 14
SATISFIED = {"satisfied_local", "satisfied_non_promotional", "satisfied_repository"}
MISSING = {
    "missing_external",
    "missing_external_publication",
    "missing_release_evidence",
}


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def load(path: pathlib.Path) -> dict[str, Any]:
    value = json.loads(
        path.read_text(),
        object_pairs_hook=reject_duplicate_keys,
        parse_constant=lambda item: (_ for _ in ()).throw(ValueError(item)),
    )
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected object")
    return value


def digest(path: pathlib.Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def collect_referenced_issues(inventory: dict[str, Any]) -> set[int]:
    """Every owner_issue named anywhere in the inventory.

    This used to be a hardcoded set of issue numbers the review prose was
    checked against, which had to be remembered and edited every time an
    owner_issue changed in the inventory — the same shape as the ADR 0012
    counter and the schema copies this repository has already fixed by
    removing the duplicate rather than promising to keep it in sync.
    """
    issues: set[int] = set()
    for fact in inventory.get("evidence_facts", []):
        if isinstance(fact, dict) and isinstance(fact.get("owner_issue"), int):
            issues.add(fact["owner_issue"])
    for section in ("completed_local_triggers", "required_stable_blockers"):
        for trigger in inventory.get(section, []):
            if isinstance(trigger, dict) and isinstance(trigger.get("owner_issue"), int):
                issues.add(trigger["owner_issue"])
    return issues


def load_issue_state(failures: list[str]) -> dict[int, str]:
    """Load the in-repo owner_issue open/closed ledger.

    Returns a map of issue number to lowercase state ("open"/"closed") for
    every issue the gate can vouch for; issues absent from the map could not
    be verified and callers must treat that as a failure, not as "open".
    """
    try:
        state = load(ISSUE_STATE_PATH)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        failures.append(f"invalid promotion blocker issue-state ledger: {error}")
        return {}
    if state.get("schema") != "casegraphen.review.promotion_blocker_issue_state.v0":
        failures.append("promotion blocker issue-state ledger schema mismatch")
    if state.get("schema_version") != 1:
        failures.append("promotion blocker issue-state ledger version mismatch")

    captured_at = state.get("captured_at")
    try:
        captured_date = datetime.date.fromisoformat(str(captured_at))
    except ValueError:
        failures.append("promotion blocker issue-state ledger has no valid captured_at date")
        captured_date = None
    if captured_date is not None:
        age_days = (datetime.date.today() - captured_date).days
        if age_days < 0:
            failures.append("promotion blocker issue-state ledger captured_at is in the future")
        elif age_days > ISSUE_STATE_MAX_AGE_DAYS:
            failures.append(
                "promotion blocker issue-state ledger is "
                f"{age_days} days old (max {ISSUE_STATE_MAX_AGE_DAYS}); "
                "refresh it with `gh issue view`"
            )

    issues = state.get("issues")
    if not isinstance(issues, dict):
        failures.append("promotion blocker issue-state ledger has no issues map")
        return {}
    result: dict[int, str] = {}
    for key, entry in issues.items():
        if not isinstance(entry, dict) or entry.get("state") not in {"open", "closed"}:
            failures.append(f"promotion blocker issue-state ledger entry is malformed: {key}")
            continue
        try:
            number = int(key)
        except ValueError:
            failures.append(f"promotion blocker issue-state ledger key is not an issue number: {key}")
            continue
        result[number] = entry["state"]
    return result


def check_owner_issue_open(
    owner: int, where: str, issue_state: dict[int, str], failures: list[str]
) -> None:
    state = issue_state.get(owner)
    if state is None:
        failures.append(
            f"{where} names Issue #{owner}, which is not recorded in the "
            "promotion blocker issue-state ledger"
        )
    elif state == "closed":
        failures.append(
            f"{where} names Issue #{owner}, which the issue-state ledger "
            "records as closed"
        )


def main() -> int:
    failures: list[str] = []
    issue_state = load_issue_state(failures)
    try:
        inventory = load(INVENTORY_PATH)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"promotion-review-conformance: invalid inventory: {error}", file=sys.stderr)
        return 1
    required_top = {
        "schema", "schema_version", "as_of_date", "review_source_commit", "decision",
        "surface", "evidence_facts", "completed_local_triggers",
        "required_stable_blockers", "optional_post_v0_enhancements",
    }
    if set(inventory) != required_top:
        failures.append("promotion inventory has missing or unknown top-level fields")
    if inventory.get("schema") != "casegraphen.review.graph_engineering_promotion_inventory.v1":
        failures.append("promotion inventory schema mismatch")
    if inventory.get("schema_version") != 1:
        failures.append("promotion inventory version mismatch")
    if re.fullmatch(r"[0-9a-f]{40}", str(inventory.get("review_source_commit"))) is None:
        failures.append("promotion review source commit is not an exact SHA")
    decision = inventory.get("decision", {})
    if decision != {"contract": "experimental-v0", "promotion_recommended": False, "accepted": False}:
        failures.append("promotion inventory must retain experimental-v0 and reject promotion")

    product = load(ROOT / "docs/product-surface.v0.json")
    contracts = load(ROOT / "schemas/experimental/contracts.v0.json")
    pilot = load(ROOT / "docs/pilots/issue-76/pilot-report.json")
    surface = inventory.get("surface", {})
    if surface.get("workflow_count") != len(product.get("workflows", [])):
        failures.append("promotion workflow count is stale")
    if surface.get("runtime_family_count") != len(pilot.get("adapters", [])):
        failures.append("promotion runtime-family count is stale")
    if surface.get("experimental_contract_version") != 0:
        failures.append("promotion contract version must describe experimental v0")
    if surface.get("experimental_contract_count") != len(contracts.get("contracts", [])):
        failures.append("promotion experimental-contract count is stale")

    facts: dict[str, dict[str, Any]] = {}
    for fact in inventory.get("evidence_facts", []):
        if not isinstance(fact, dict) or not isinstance(fact.get("id"), str):
            failures.append("promotion evidence fact is malformed")
            continue
        if fact["id"] in facts:
            failures.append(f"duplicate promotion evidence fact: {fact['id']}")
        facts[fact["id"]] = fact
        status = fact.get("status")
        if status not in SATISFIED | MISSING:
            failures.append(f"promotion evidence fact has unknown status: {fact['id']}")
            continue
        if status in SATISFIED:
            reference = fact.get("reference")
            if not isinstance(reference, dict) or set(reference) != {"path", "content_hash"}:
                failures.append(f"satisfied fact has no exact retained reference: {fact['id']}")
                continue
            relative = pathlib.PurePosixPath(str(reference["path"]))
            if relative.is_absolute() or ".." in relative.parts:
                failures.append(f"unsafe retained reference: {fact['id']}")
                continue
            path = ROOT.joinpath(*relative.parts)
            if not path.is_file() or path.is_symlink():
                failures.append(f"retained reference is absent or unsafe: {fact['id']}")
            elif reference.get("content_hash") != digest(path):
                failures.append(f"retained reference hash drifted: {fact['id']}")
            if re.fullmatch(r"\d{4}-\d{2}-\d{2}", str(fact.get("evidence_date"))) is None:
                failures.append(f"satisfied fact has no evidence date: {fact['id']}")
            if status != "satisfied_repository" and re.fullmatch(
                r"[0-9a-f]{40}", str(fact.get("evidence_commit"))
            ) is None:
                failures.append(f"executed evidence has no exact evaluated commit: {fact['id']}")
        elif not isinstance(fact.get("owner_issue"), int):
            failures.append(f"missing evidence fact has no owner Issue: {fact['id']}")
        else:
            check_owner_issue_open(
                fact["owner_issue"], f"missing evidence fact {fact['id']}", issue_state, failures
            )

    completed_ids: set[str] = set()
    for trigger in inventory.get("completed_local_triggers", []):
        trigger_id = trigger.get("id") if isinstance(trigger, dict) else None
        if not isinstance(trigger_id, str) or trigger_id in completed_ids:
            failures.append("completed promotion trigger is malformed or duplicated")
            continue
        completed_ids.add(trigger_id)
        fact_ids = trigger.get("fact_ids")
        if not isinstance(fact_ids, list) or not fact_ids:
            failures.append(f"completed trigger has no facts: {trigger_id}")
        elif not all(facts.get(item, {}).get("status") in SATISFIED for item in fact_ids):
            failures.append(f"completed trigger is not proved by retained evidence: {trigger_id}")

    blocker_ids: set[str] = set()
    for trigger in inventory.get("required_stable_blockers", []):
        trigger_id = trigger.get("id") if isinstance(trigger, dict) else None
        if not isinstance(trigger_id, str) or trigger_id in blocker_ids:
            failures.append("stable blocker is malformed or duplicated")
            continue
        blocker_ids.add(trigger_id)
        owner = trigger.get("owner_issue")
        if not isinstance(owner, int):
            failures.append(f"stable blocker has no owner Issue: {trigger_id}")
        else:
            check_owner_issue_open(owner, f"stable blocker {trigger_id}", issue_state, failures)
        fact_ids = trigger.get("fact_ids")
        if not isinstance(fact_ids, list) or not fact_ids or any(item not in facts for item in fact_ids):
            failures.append(f"stable blocker has unknown facts: {trigger_id}")
        elif all(facts[item].get("status") in SATISFIED for item in fact_ids):
            failures.append(
                f"next-trigger item is already satisfied by retained evidence: {trigger_id}"
            )
    if completed_ids & blocker_ids:
        failures.append("the same trigger is both completed and outstanding")
    enhancements = inventory.get("optional_post_v0_enhancements")
    if (
        not isinstance(enhancements, list)
        or not enhancements
        or any(not isinstance(item, str) or not item for item in enhancements)
        or len(set(enhancements)) != len(enhancements)
    ):
        failures.append("optional post-v0 enhancements are malformed or duplicated")

    review = REVIEW_PATH.read_text()
    for trigger_id in sorted(completed_ids | blocker_ids):
        if f"promotion-trigger:{trigger_id}" not in review:
            failures.append(f"promotion review omits structured trigger marker: {trigger_id}")
    for issue in sorted(collect_referenced_issues(inventory)):
        if f"#{issue}" not in review:
            failures.append(f"promotion review omits Issue #{issue}")
    if "promotion_recommended: false" not in review:
        failures.append("promotion review omits the explicit false recommendation")
    if "Required stable-promotion blockers" not in review or "Optional post-v0 enhancements" not in review:
        failures.append("promotion review does not separate blockers from optional enhancements")

    if failures:
        for failure in failures:
            print(f"promotion-review-conformance: FAIL {failure}", file=sys.stderr)
        return 1
    print(
        "promotion-review-conformance: ok "
        f"({len(completed_ids)} completed local triggers, {len(blocker_ids)} open blockers)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

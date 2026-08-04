#!/usr/bin/env python3
"""Deterministic inventory and link conformance for CaseGraphen ADRs."""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
from urllib.parse import unquote


FILENAME = re.compile(r"^(?P<id>[0-9]{4})-[a-z0-9]+(?:-[a-z0-9]+)*\.md$")
HEADING = re.compile(r"^# ADR (?P<id>[0-9]{4}): (?P<title>\S.*)$")
MARKDOWN_LINK = re.compile(r"(?<!!)\[[^\]]+\]\((?P<target>[^)]+)\)")
REFERENCE_LINK = re.compile(
    r"^[ \t]{0,3}\[[^\]]+\]:[ \t]*(?P<target>\S+)", re.MULTILINE
)


def inventory(adr_dir: pathlib.Path) -> tuple[dict[int, pathlib.Path], list[str]]:
    failures: list[str] = []
    by_id: dict[int, pathlib.Path] = {}
    markdown_files = sorted(
        (path for path in adr_dir.rglob("*") if path.is_file()),
        key=lambda path: str(path),
    )
    if not markdown_files:
        return {}, [f"{adr_dir}: no ADR Markdown files"]

    for path in markdown_files:
        filename_match = FILENAME.fullmatch(path.name)
        if filename_match is None:
            failures.append(f"{path}: filename must be NNNN-lowercase-slug.md")
            continue
        filename_id = int(filename_match.group("id"))
        try:
            first_line = path.read_text(encoding="utf-8").splitlines()[0]
        except (OSError, IndexError) as error:
            failures.append(f"{path}: cannot read ADR heading: {error}")
            continue
        heading_match = HEADING.fullmatch(first_line)
        if heading_match is None:
            failures.append(f"{path}: first line must be '# ADR NNNN: Title'")
            continue
        heading_id = int(heading_match.group("id"))
        if filename_id != heading_id:
            failures.append(
                f"{path}: filename ADR {filename_id:04d} does not match "
                f"heading ADR {heading_id:04d}"
            )
        prior = by_id.get(heading_id)
        if prior is not None:
            failures.append(
                f"duplicate ADR {heading_id:04d}: {prior.name}, {path.name}"
            )
        else:
            by_id[heading_id] = path

    if by_id:
        expected = set(range(1, max(by_id) + 1))
        missing = sorted(expected.difference(by_id))
        if missing:
            failures.append(
                "missing ADR identifiers: "
                + ", ".join(f"{identifier:04d}" for identifier in missing)
            )
    return by_id, failures


def adr_links(markdown_root: pathlib.Path, adr_dir: pathlib.Path) -> list[str]:
    failures: list[str] = []
    resolved_adr_dir = adr_dir.resolve()
    excluded_fixture_root = (markdown_root / "tests/fixtures").resolve()
    for source in sorted(markdown_root.rglob("*.md"), key=lambda path: str(path)):
        resolved_source = source.resolve()
        relative_parts = source.relative_to(markdown_root).parts
        if any(part in {".git", "target"} for part in relative_parts):
            continue
        if excluded_fixture_root in resolved_source.parents:
            continue
        try:
            content = source.read_text(encoding="utf-8")
        except OSError as error:
            failures.append(f"{source}: cannot inspect Markdown links: {error}")
            continue
        links = list(MARKDOWN_LINK.finditer(content)) + list(REFERENCE_LINK.finditer(content))
        for match in sorted(links, key=lambda item: item.start()):
            raw_target = match.group("target").strip()
            # Optional Markdown titles are outside the path and do not affect
            # ADR identity. Angle brackets only quote a path containing spaces.
            if raw_target.startswith("<") and ">" in raw_target:
                raw_target = raw_target[1 : raw_target.index(">")]
            else:
                raw_target = raw_target.split(maxsplit=1)[0]
            target_without_fragment = unquote(raw_target.split("#", 1)[0])
            if not target_without_fragment or "://" in target_without_fragment:
                continue
            candidate = (source.parent / target_without_fragment).resolve()
            if resolved_adr_dir not in candidate.parents:
                continue
            if not candidate.is_file():
                line = content.count("\n", 0, match.start()) + 1
                failures.append(
                    f"{source}:{line}: broken ADR link {match.group('target')}"
                )
    return failures


def next_identifier_documentation(index: pathlib.Path, expected: int) -> list[str]:
    try:
        content = index.read_text(encoding="utf-8")
    except OSError as error:
        return [f"{index}: cannot read ADR index: {error}"]
    matches = re.findall(r"next available\s+identifier is \*\*([0-9]{4})\*\*", content, re.I)
    if matches != [f"{expected:04d}"]:
        return [
            f"{index}: document exactly one next available identifier {expected:04d}"
        ]
    return []


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--adr-dir", type=pathlib.Path, default=pathlib.Path("docs/adr"))
    parser.add_argument("--markdown-root", type=pathlib.Path, default=pathlib.Path("."))
    parser.add_argument("--index", type=pathlib.Path)
    args = parser.parse_args()

    by_id, failures = inventory(args.adr_dir)
    failures.extend(adr_links(args.markdown_root, args.adr_dir))
    next_identifier = max(by_id, default=0) + 1
    if args.index is not None:
        failures.extend(next_identifier_documentation(args.index, next_identifier))
    if failures:
        for failure in sorted(failures):
            print(f"adr-conformance: FAIL {failure}", file=sys.stderr)
        return 1
    print(
        f"adr-conformance: ok ({len(by_id)} ADRs, next {next_identifier:04d})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

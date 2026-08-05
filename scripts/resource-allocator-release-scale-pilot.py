#!/usr/bin/env python3
"""Run and independently envelope the real 10k/100k allocator API pilot."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import subprocess
import tempfile
import time
from pathlib import Path


RSS_LIMITS = {10_000: 2 * 1024**3, 100_000: 8 * 1024**3}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command_version(*command: str) -> str:
    return subprocess.check_output(command, text=True).strip()


def rss_bytes(pid: int) -> int | None:
    proc_status = Path(f"/proc/{pid}/status")
    if proc_status.is_file():
        for line in proc_status.read_text(encoding="utf-8").splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) * 1024
    try:
        observed = subprocess.run(
            ["ps", "-o", "rss=", "-p", str(pid)],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        return None
    if observed.returncode == 0 and observed.stdout.strip():
        return int(observed.stdout.strip()) * 1024
    return None


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=repo, text=True).strip()


def run(
    repo: Path,
    binary: Path,
    target: int,
    output: Path,
    evidence_class: str,
    require_clean_source: bool,
) -> dict[str, object]:
    if target not in RSS_LIMITS:
        raise ValueError("release-scale target must be 10000 or 100000")
    source_dirty = bool(git(repo, "status", "--porcelain", "--untracked-files=no"))
    if require_clean_source and source_dirty:
        raise RuntimeError("release-scale evidence requires an exact clean source revision")
    with tempfile.TemporaryDirectory(prefix="casegraphen-allocator-scale-") as directory:
        raw = Path(directory) / "raw-report.json"
        environment = dict(os.environ)
        environment["CASEGRAPHEN_ALLOCATOR_EVENT_TARGET"] = str(target)
        started = time.monotonic_ns()
        process = subprocess.Popen(
            [str(binary), str(raw)],
            cwd=repo,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        peak_rss = 0
        sample_count = 0
        try:
            while process.poll() is None:
                observed = rss_bytes(process.pid)
                if observed is not None:
                    peak_rss = max(peak_rss, observed)
                    sample_count += 1
                # Process-spawning RSS probes are materially expensive on
                # macOS. A half-second cadence still observes the long-running
                # release lanes without making the harness the main workload.
                time.sleep(0.5)
        except BaseException:
            process.terminate()
            process.wait(timeout=10)
            raise
        stdout, stderr = process.communicate()
        elapsed_ms = (time.monotonic_ns() - started) // 1_000_000
        if process.returncode != 0:
            raise RuntimeError(
                f"allocator pilot failed ({process.returncode})\nstdout:\n{stdout}\nstderr:\n{stderr}"
            )
        report = json.loads(raw.read_text(encoding="utf-8"))

    memory_limit = RSS_LIMITS[target]
    release_passed = bool(
        report["passed"]
        and report["journal_event_count"] == target
        and report["checkpoint_compaction"]["full_replay_equivalent"] is True
        and report["crash_before_publication_ignored"] is True
        and report["crash_after_publication_refused"] is True
        and peak_rss > 0
        and peak_rss <= memory_limit
    )
    report.update(
        passed=release_passed,
        accepted=False,
        observed_peak_rss_bytes=peak_rss,
        observed_peak_rss_threshold_bytes=memory_limit,
        release_scale_envelope={
            "schema": "casegraphen.experimental.resource_allocator_release_scale.v0",
            "schema_version": 0,
            "real_public_allocator_api": True,
            "long_lived_allocator_instance": True,
            "rss_method": "proc_status_or_ps_rss_50ms_sampling",
            "rss_sample_count": sample_count,
            "nonzero_rss_required": True,
            "elapsed_ms": elapsed_ms,
            "source_revision": git(repo, "rev-parse", "HEAD"),
            "source_worktree_dirty": source_dirty,
            "evidence_class": evidence_class,
            "promotion_authority": False,
            "attestation_status": "not_attested",
            "binary_sha256": digest(binary),
            "harness_sha256": digest(Path(__file__)),
            "allocator_source_sha256": digest(repo / "src/resource_allocator.rs"),
            "pilot_source_sha256": digest(
                repo / "examples/resource_allocator_durability_pilot.rs"
            ),
            "python": platform.python_version(),
            "platform": platform.platform(),
            "machine": platform.machine(),
            "host_identity_sha256": hashlib.sha256(platform.node().encode()).hexdigest(),
            "rustc_verbose": command_version("rustc", "--version", "--verbose"),
        },
        review_disposition="operator_review_required",
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_suffix(output.suffix + ".tmp")
    temporary.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, output)
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--event-target", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--evidence-class",
        choices=("local-observation", "release-candidate"),
        default="local-observation",
    )
    parser.add_argument("--require-clean-source", action="store_true")
    args = parser.parse_args()
    report = run(
        args.repo.resolve(),
        args.binary.resolve(),
        args.event_target,
        args.output.resolve(),
        args.evidence_class,
        args.require_clean_source,
    )
    print(
        json.dumps(
            {
                "event_target": report["event_threshold"],
                "passed": report["passed"],
                "peak_rss_bytes": report["observed_peak_rss_bytes"],
            },
            sort_keys=True,
        )
    )
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Non-model runner used only to exercise harness isolation and capture."""

import json
import os
import pathlib
import sys

workspace = pathlib.Path.cwd().resolve()
assert pathlib.Path(os.environ["CASEGRAPHEN_EVAL_WORKSPACE"]).resolve() == workspace
skill = pathlib.Path(os.environ["CASEGRAPHEN_EVAL_SKILL"]).resolve()
assert skill.is_relative_to(workspace)
assert (skill / "SKILL.md").is_file()
assert (workspace / "TASK.md").is_file()
assert (workspace / "case-observation.json").is_file()
assert "Determine the safe next operation" in sys.stdin.read()
(workspace / "operation.decision.json").write_text(
    json.dumps(
        {
            "evidence_status": "unreviewed",
            "accepted": False,
            "next_operation": "request_independent_review",
            "mutation_executed": False,
        }
    )
    + "\n"
)
print(json.dumps({"runner": "fake", "workspace_isolated": True}))

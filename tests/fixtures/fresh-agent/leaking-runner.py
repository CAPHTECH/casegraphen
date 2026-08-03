#!/usr/bin/env python3
"""Fixture that proves generated credential material is withheld."""

import json
import os
import pathlib

workspace = pathlib.Path.cwd()
(workspace / "operation.decision.json").write_text(
    json.dumps({"evidence_status": "unreviewed", "accepted": False}) + "\n"
)
(workspace / "provider-debug.txt").write_text(os.environ["TEST_SECRET"])
print(os.environ["TEST_SECRET"])

#!/usr/bin/env python3
"""Adversarial fixture that copies a conventional disk session into output."""

import os
import pathlib

workspace = pathlib.Path(os.environ["CASEGRAPHEN_EVAL_WORKSPACE"])
credential = pathlib.Path.home() / ".codex" / "auth.json"
(workspace / "disk-session-copy.txt").write_bytes(credential.read_bytes())
(workspace / "evidence-review.json").write_text('{"accepted":false,"review_status":"unreviewed"}\n')
print(credential.read_text())

#!/usr/bin/env python3
"""Refresh stale evidence digests in the capability catalogue.

Replaces only the sha256 values of evidence entries whose file changed, then
regenerates the derived matrix with the tracked capability-matrix tool. Prints
each refreshed evidence id. Exit 0 when the catalogue check passes.
"""
import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CATALOGUE = ROOT / "tests/capabilities/catalog-v1.json"


def stale(node, out):
    if isinstance(node, dict):
        if str(node.get("id", "")).startswith("e-") and "path" in node and "sha256" in node:
            path = ROOT / node["path"]
            if path.is_file():
                actual = hashlib.sha256(path.read_bytes()).hexdigest()
                if actual != node["sha256"]:
                    out.append((node["id"], node["sha256"], actual))
        for value in node.values():
            stale(value, out)
    elif isinstance(node, list):
        for value in node:
            stale(value, out)


def cargo_matrix(flag):
    return subprocess.run(
        ["cargo", "run", "--locked", "-q", "-p", "sf-conformance",
         "--bin", "capability-matrix", "--", flag],
        cwd=ROOT, capture_output=True, text=True,
    )


def main():
    raw = CATALOGUE.read_text()
    changes = []
    stale(json.loads(raw), changes)
    for evidence, old, new in changes:
        if raw.count(old) != 1:
            sys.exit(f"digest for {evidence} is not unique; refusing to edit")
        raw = raw.replace(old, new)
        print(f"refreshed {evidence}")
    if changes:
        CATALOGUE.write_text(raw)
    generated = cargo_matrix("--generate")
    if generated.returncode != 0:
        sys.exit(generated.stderr.strip().splitlines()[-1] if generated.stderr else "generate failed")
    checked = cargo_matrix("--check")
    last = (checked.stdout or checked.stderr).strip().splitlines()
    print(last[-1] if last else "")
    sys.exit(checked.returncode)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Bind real Rust reports to this checkout and the downstream source patch."""
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPORTS = ("lcov.info", "clippy.json", "python-coverage.xml")


def identity():
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    if head != os.environ["SOURCE_SHA"]:
        raise SystemExit("Coverage checkout differs from requested source")
    return {
        "head": head,
        "base": os.environ["BASE_SHA"],
        "patch_manifest": hashlib.sha256((ROOT / "patches/hbb-common/manifest.json").read_bytes()).hexdigest(),
        "toolchain": "1.91.1",
        "features": "workspace/all-features",
        "reports": {name: hashlib.sha256((ROOT / "artifacts" / name).read_bytes()).hexdigest() for name in REPORTS},
    }


def main():
    evidence = identity()
    report = ROOT / "artifacts/lcov.info"
    paths = [line[3:] for line in report.read_text().splitlines() if line.startswith("SF:")]
    if not paths or not any(path.startswith("src/") for path in paths):
        raise SystemExit("Missing measured server source coverage")
    for path in paths:
        resolved = (ROOT / path).resolve()
        if not resolved.is_relative_to(ROOT) or not resolved.is_file():
            raise SystemExit(f"Coverage path is outside this checkout or missing: {path}")
    output = ROOT / "artifacts/source.json"
    if sys.argv[1] == "write":
        output.write_text(json.dumps(evidence, indent=2) + "\n")
    elif sys.argv[1] == "verify":
        if json.loads(output.read_text()) != evidence:
            raise SystemExit("Coverage provenance or report digest mismatch")
    else:
        raise SystemExit("Expected write or verify")


if __name__ == "__main__":
    main()

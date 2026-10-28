"""Validate generated command schemas with an independent JSON Schema implementation."""
import argparse
import json
import os
from pathlib import Path
import subprocess

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release" / ("raytrace-rs.exe" if os.name == "nt" else "raytrace-rs"))
    args = parser.parse_args()
    schema = json.loads(subprocess.check_output([str(args.binary), "commands"], encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    validate = Draft202012Validator(schema)
    examples = json.loads((ROOT / "examples/automation_assembly.json").read_text(encoding="utf-8"))["commands"]
    examples += [
        {"expected_revision": 0, "command": "set_property", "target": "project", "change": {"property": "name", "value": "Example"}},
        {"command": "undo"},
        {"command": "query", "query": {"query": "materials", "target": "c2"}},
        {"command": "open", "path": "lens.zmx", "catalogs": ["glass"], "cad_deflection_mm": 0.02},
        {"command": "convergence", "plan": "plan.json", "output": "results"},
    ]
    for value in examples:
        validate.validate(value)
    rejected = [
        {"command": "unknown"},
        {"command": "undo", "extra": True},
        {"command": "trace", "extra": True},
        {"expected_revision": "0", "command": "undo"},
        {"command": "set_property", "target": "project", "change": {"property": "max_depth", "value": "16"}},
        {"command": "set_property", "target": "c2", "change": {"property": "position_mm", "value": [0, 0]}},
        {"command": "transaction", "label": "bad", "edits": [{"command": "undo", "extra": True}]},
    ]
    for value in rejected:
        if not list(validate.iter_errors(value)):
            raise ValueError(f"Schema accepted an invalid request: {value}")
    print(f"Command schema validated: {len(examples)} valid requests and {len(rejected)} rejected requests")


if __name__ == "__main__":
    main()

"""Check the Git index before committing/publishing. Reads staged bytes, not local data."""
import json
from pathlib import Path, PurePosixPath
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
FILES = {
    ".gitattributes", ".gitignore", "Cargo.toml", "Cargo.lock", "README.md",
    "Raytraceを起動.cmd", "examples/exchange_check.rs",
    "scripts/check_publish.py", "scripts/step_mesh.py", "scripts/zemax_exchange_probe.py",
}
DEMOS = {
    "examples/sequential_lens.json", "examples/nonsequential_slab.json",
    "examples/integrator_rod.json", "examples/folded_mirror.json",
}


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


def embedded_document(value):
    if isinstance(value, dict):
        if value.get("format") == "raytrace-project-v1" or value.get("zemax") is not None:
            return True
        return any(embedded_document(v) for v in value.values())
    if isinstance(value, list):
        return any(embedded_document(v) for v in value)
    return False


def main():
    entries = git("ls-files", "--stage", "-z").split(b"\0")
    failures, checked = [], 0
    for entry in filter(None, entries):
        metadata, name = entry.split(b"\t", 1)
        mode, _, stage = metadata.split()
        path = name.decode("utf-8")
        part = PurePosixPath(path)
        source = part.suffix == ".rs" and part.parts[0] in {"src", "tests"}
        if mode != b"100644" or stage != b"0" or not (path in FILES | DEMOS or source):
            failures.append(f"Unapproved publication path or mode: {path}")
            continue
        content = git("show", f":{path}")
        if b"\0" in content:
            failures.append(f"Binary content: {path}")
            continue
        try:
            text = content.decode("utf-8")
            if path in DEMOS and embedded_document(json.loads(text)):
                failures.append(f"Embedded original project data: {path}")
        except (UnicodeDecodeError, json.JSONDecodeError):
            failures.append(f"Invalid text/demo: {path}")
        checked += 1
    if failures or checked == 0:
        for failure in failures:
            print(failure, file=sys.stderr)
        return 1
    print(f"Publication index checked: {checked} source/documentation/demo files; no vendor files.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

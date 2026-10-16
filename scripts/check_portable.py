"""Verify a Windows ZIP and exercise owned models with only Windows on PATH."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import zipfile

from check_publish import embedded_document
from package_windows import FILES, ROOT


def check(archive, output):
    if os.name != "nt":
        raise ValueError("This acceptance check targets Windows")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    prefix = f"raytrace-rs-{version}-windows-x64/"
    allowed = set(FILES.values()) | {"package.json", "START_HERE.md", "THIRD_PARTY_LICENSES.txt"}
    output.parent.mkdir(parents=True, exist_ok=True)
    checks = []
    with tempfile.TemporaryDirectory(prefix="portable_", dir=output.parent) as temporary:
        destination = Path(temporary)
        with zipfile.ZipFile(archive) as package:
            if len(package.namelist()) != len(allowed) or set(package.namelist()) != {prefix + n for n in allowed}:
                raise ValueError("Unexpected package members")
            if package.testzip() is not None:
                raise ValueError("ZIP integrity check failed")
            manifest = json.loads(package.read(prefix + "package.json"))
            if manifest["version"] != version or manifest["platform"] != "windows-x64":
                raise ValueError("Package version/platform mismatch")
            if set(manifest["sha256"]) != set(FILES.values()):
                raise ValueError("Incomplete package checksums")
            for name, digest in manifest["sha256"].items():
                if hashlib.sha256(package.read(prefix + name)).hexdigest() != digest:
                    raise ValueError(f"Package checksum mismatch: {name}")
                if name.startswith("examples/") and embedded_document(json.loads(package.read(prefix + name))):
                    raise ValueError("Example contains an original optical document")
            package.extractall(destination)
        root = destination / prefix.rstrip("/")
        environment = os.environ.copy()
        system = Path(environment.get("SystemRoot", "C:/Windows"))
        environment["PATH"] = os.pathsep.join([str(system / "System32"), str(system)])
        environment["RAYTRACE_DATA_DIR"] = str(destination / "runtime")
        environment.pop("RAYTRACE_FREECAD_PYTHON", None)
        environment.pop("RAYTRACE_ZEMAX_ROOT", None)

        def run(binary, *args):
            result = subprocess.run([str(root / (binary + ".exe")), *map(str, args)],
                                    cwd=root, env=environment, capture_output=True,
                                    encoding="utf-8", errors="replace", timeout=90)
            if result.returncode:
                raise ValueError(f"{binary} failed: {result.stderr}")
            return result.stdout

        def record(name):
            return json.loads((root / name).read_text(encoding="utf-8"))

        for binary in ["raytrace-rs", "raytrace-gui"]:
            if run(binary, "--version").strip() != f"{binary} {version}":
                raise ValueError("Executable version mismatch")
        checks.append("GUI/CLI version without development tools on PATH")
        for sample in sorted((root / "examples").glob("*.json")):
            name = sample.stem + ".run.json"
            run("raytrace-rs", sample, "--rays-per-source", 256, "--threads", 2, "--output", name)
            data = record(name)
            if abs(data["relative_budget_closure_error"]) > 1e-10:
                raise ValueError("Sample power budget failed")
            checks.append("Trace owned sample: " + sample.stem)

        # A fresh native file verifies parsing and source-file-independent replay.
        native = root / "owned.zmx"
        native.write_text("MODE SEQ\nUNIT MM\nENPD 2\nWAVM 1 .55 1\nSURF 0\nTYPE STANDARD\nDISZ INFINITY\nSURF 1\nTYPE STANDARD\nSTOP\nDIAM 1\nDISZ 10\nSURF 2\nTYPE STANDARD\nDIAM 2\nDISZ 0\n", encoding="utf-8")
        run("raytrace-rs", native, "--inspect", "--output", "owned.project.json")
        project = record("owned.project.json")
        project["scene"]["surfaces"][-1]["pose"]["translation_mm"][2] = 12.0
        (root / "moved.project.json").write_text(json.dumps(project), encoding="utf-8")
        native.unlink()
        run("raytrace-rs", "moved.project.json", "--export-zmx", "moved.zmx", "--threads", 1, "--output", "moved.run.json")
        run("raytrace-rs", "moved.zmx", "--threads", 1, "--output", "native.run.json")
        if abs(record("native.run.json")["report"]["budget"]["detected"] - 1.0) > 1e-10:
            raise ValueError("Native roundtrip lost power")
        (root / "moved.project.json").unlink()
        (root / "moved.zmx").unlink()
        run("raytrace-rs", "--replay", "moved.run.json", "--threads", 2, "--output", "replay.run.json")
        if record("moved.run.json")["report"] != record("replay.run.json")["report"]:
            raise ValueError("Saved calculation replay differs")
        checks.extend(["Native ZMX import, placement edit, JSON save, ZMX roundtrip",
                       "Frozen calculation replay after original inputs are removed"])
    result = {"version": version, "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
              "checks": checks, "passed": True,
              "scope": "Windows CLI workflows and GUI version entry point with only Windows directories on PATH. Interactive GUI rendering and FreeCAD conversion need separate checks."}
    output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(f"Portable acceptance: {len(checks)} checks passed; {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "results/portable_summary.json")
    options = parser.parse_args()
    check(options.archive.resolve(), options.output.resolve())

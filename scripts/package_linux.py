"""Build the static Linux x64 CLI archive and smoke-test the extracted copy.

The archive holds the musl-linked `raytrace-rs`, the owned examples, JSON schemas,
documentation and license notices. Run on Linux with the
`x86_64-unknown-linux-musl` Rust target installed.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib
from check_publish import embedded_document
from package_windows import dependency_notices

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-unknown-linux-musl"
BINARY = ROOT / "target" / TARGET / "release" / "raytrace-rs"
DOCUMENTS = ["README.md", "LICENSE", "docs/FEATURES.md", "docs/GUIDE.md", "docs/DEVELOPMENT.md"]
EXAMPLES = ["examples/sequential_lens.json", "examples/nonsequential_slab.json",
            "examples/integrator_rod.json", "examples/folded_mirror.json"]


def files():
    listed = {name: ROOT / name for name in DOCUMENTS + EXAMPLES}
    for schema in sorted((ROOT / "schemas").glob("*.schema.json")):
        listed[f"schemas/{schema.name}"] = schema
    return listed


def build_archive(version, output):
    name = f"raytrace-rs-{version}-linux-x64"
    hashes = {}
    path = output / f"{name}.tar.gz"
    with tarfile.open(path, "w:gz") as archive:
        def add(member, data, mode=0o644):
            info = tarfile.TarInfo(f"{name}/{member}")
            info.size, info.mode, info.mtime = len(data), mode, 0
            archive.addfile(info, io.BytesIO(data))
            hashes[member] = hashlib.sha256(data).hexdigest()

        add("raytrace-rs", BINARY.read_bytes(), 0o755)
        for member, source in files().items():
            data = source.read_bytes()
            if member.startswith("examples/") and embedded_document(json.loads(data)):
                raise ValueError("An example contains an embedded original document")
            add(member, data)
        add("THIRD_PARTY_LICENSES.txt", dependency_notices(TARGET, "raytrace-cli", assets=False))
        package = {"version": version, "license": "MIT", "platform": "linux-x64",
                   "target": TARGET, "sha256": hashes}
        add("package.json", json.dumps(package, indent=2).encode())
    return path, name


def run(cli, *args, stdin=None, cwd):
    result = subprocess.run([str(cli), *map(str, args)], input=stdin, cwd=cwd,
                            capture_output=True, text=True, encoding="utf-8")
    responses = [json.loads(line) for line in result.stdout.splitlines()]
    failed = [r for r in responses if not r.get("ok")]
    if result.returncode != 0 or not responses or failed:
        raise RuntimeError(f"{args}: exit {result.returncode}\n{result.stdout}\n{result.stderr}")
    return responses


def smoke(path, name, version):
    """Extract into an empty directory and use only the archive contents."""
    with tempfile.TemporaryDirectory() as temp:
        temp = Path(temp)
        with tarfile.open(path) as archive:
            archive.extractall(temp, filter="data")
        root = temp / name
        cli = root / "raytrace-rs"
        reported = subprocess.check_output([str(cli), "--version"], text=True).strip()
        if reported != f"raytrace-rs {version}":
            raise ValueError(f"Unexpected version: {reported}")
        for example in EXAMPLES:
            stem = Path(example).stem
            run(cli, "trace", f"model.path={example}", "overrides.rays_per_source:=256",
                "threads:=2", f"output=results/{stem}.json", cwd=root)
        run(cli, "replay", "record.path=results/sequential_lens.json", "output=results/replay.json", cwd=root)
        requests = "\n".join(json.dumps(r) for r in [
            {"command": "version"},
            {"command": "inspect", "model": {"path": "examples/integrator_rod.json"}},
            {"command": "export_screen", "record": {"path": "results/integrator_rod.json"},
             "output": "results/screen"},
        ])
        run(cli, "run", "-", stdin=requests, cwd=root)
        if not (root / "results/screen/screen.png").is_file():
            raise ValueError("Screen export did not write a PNG")
        for schema in (root / "schemas").glob("*.schema.json"):
            json.loads(schema.read_text(encoding="utf-8"))
    print(f"Smoke-tested {len(EXAMPLES)} examples, replay and JSON requests from {path.name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "results/packages")
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "--locked", "-p", "raytrace-cli",
                        "--target", TARGET], cwd=ROOT, check=True)
    args.output.mkdir(parents=True, exist_ok=True)
    path, name = build_archive(version, args.output)
    smoke(path, name, version)
    print(path.resolve())


if __name__ == "__main__":
    main()

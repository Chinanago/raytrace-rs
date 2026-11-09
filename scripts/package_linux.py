"""Build the static Linux x64 CLI archive and smoke-test the extracted copy.

The archive holds the musl-linked `raytrace-rs`, the owned examples, JSON schemas,
documentation and license notices. Run on Linux with the
`x86_64-unknown-linux-musl` Rust target installed.
"""
import argparse
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib

from package_windows import dependency_notices
from smoke import smoke_package
from unix_archive import build_archive

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-unknown-linux-musl"
BINARY = ROOT / "target" / TARGET / "release" / "raytrace-rs"


def smoke(path, name, version):
    """Extract into an empty directory and use only the archive contents."""
    with tempfile.TemporaryDirectory() as temp:
        with tarfile.open(path) as archive:
            archive.extractall(temp, filter="data")
        smoke_package(Path(temp) / name, "raytrace-rs", version)
    print(f"Smoke-tested {path.name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "results/packages")
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "--locked", "-p", "raytrace-cli",
                        "--target", TARGET], cwd=ROOT, check=True)
    # The notices read `cargo metadata --offline` for the whole workspace.
    subprocess.run(["cargo", "fetch", "--locked"], cwd=ROOT, check=True)
    args.output.mkdir(parents=True, exist_ok=True)
    name = f"raytrace-rs-{version}-linux-x64"
    path = build_archive(args.output, name, version, "linux-x64", {"raytrace-rs": BINARY},
                         dependency_notices(TARGET, "raytrace-cli", assets=False), target=TARGET)
    smoke(path, name, version)
    print(path.resolve())


if __name__ == "__main__":
    main()

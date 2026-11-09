"""Build the universal macOS archive (CLI and GUI) and smoke-test the extracted copy.

Builds `raytrace-rs` and `raytrace-gui` for `aarch64-apple-darwin` and `x86_64-apple-darwin`,
joins each pair with `lipo -create`, ad-hoc signs the result and packs
`raytrace-rs-<version>-macos.tar.gz`. Run on macOS with both Rust targets installed:

    rustup target add aarch64-apple-darwin x86_64-apple-darwin
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

from package_windows import dependency_notices
from smoke import check_version, smoke_package
from unix_archive import build_archive

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin"]
BINARIES = ["raytrace-rs", "raytrace-gui"]
UNIVERSAL = ROOT / "target" / "universal-apple-darwin" / "release"
# Oldest macOS the binaries are built for (the arm64 baseline, so both slices agree).
DEPLOYMENT_TARGET = "11.0"


def build():
    environment = {**os.environ, "MACOSX_DEPLOYMENT_TARGET": DEPLOYMENT_TARGET}
    for target in TARGETS:
        subprocess.run(["cargo", "build", "--release", "--locked", "-p", "raytrace-cli",
                        "-p", "raytrace-gui", "--target", target],
                       cwd=ROOT, env=environment, check=True)


def lipo_architectures(binary):
    """The architectures `lipo -info` reports for `binary`."""
    info = subprocess.check_output(["lipo", "-info", str(binary)], text=True).strip()
    return set(info.rsplit(":", 1)[1].split())


def make_universal():
    """Join the two thin builds of each binary and return {name: universal path}."""
    UNIVERSAL.mkdir(parents=True, exist_ok=True)
    joined = {}
    for name in BINARIES:
        thin = [ROOT / "target" / target / "release" / name for target in TARGETS]
        for path in thin:
            if not path.is_file():
                raise ValueError(f"Missing build output: {path}")
        output = UNIVERSAL / name
        output.unlink(missing_ok=True)
        subprocess.run(["lipo", "-create", "-output", str(output), *map(str, thin)], check=True)
        # Apple Silicon only runs signed code. The linker signs the thin binaries ad hoc;
        # sign the joined file again so the signature covers the file as shipped.
        if shutil.which("codesign"):
            subprocess.run(["codesign", "--force", "--sign", "-", str(output)], check=True)
        if lipo_architectures(output) != {"arm64", "x86_64"}:
            raise ValueError(f"{name} is not a universal binary")
        joined[name] = output
    return joined


def smoke(path, name, version):
    """Extract into an empty directory and use only the archive contents."""
    with tempfile.TemporaryDirectory() as temp:
        with tarfile.open(path) as archive:
            archive.extractall(temp, filter="data")
        root = Path(temp) / name
        for binary in BINARIES:
            if lipo_architectures(root / binary) != {"arm64", "x86_64"}:
                raise ValueError(f"{binary} lost an architecture in the archive")
        check_version(root / "raytrace-gui", "raytrace-gui", version)
        smoke_package(root, "raytrace-rs", version)
    print(f"Smoke-tested {path.name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path, default=ROOT / "results/packages")
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    if not args.no_build:
        build()
    binaries = make_universal()
    args.output.mkdir(parents=True, exist_ok=True)
    name = f"raytrace-rs-{version}-macos"
    notices = dependency_notices(TARGETS, ["raytrace-cli", "raytrace-gui"])
    path = build_archive(args.output, name, version, "macos-universal", binaries, notices,
                         targets=TARGETS, minimum_macos=DEPLOYMENT_TARGET)
    smoke(path, name, version)
    print(path.resolve())


if __name__ == "__main__":
    main()

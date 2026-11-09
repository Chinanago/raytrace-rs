"""Shared tar.gz layout of the Linux and macOS packages."""
import hashlib
import io
import json
from pathlib import Path
import tarfile

from check_publish import embedded_document

ROOT = Path(__file__).resolve().parents[1]
DOCUMENTS = ["README.md", "LICENSE", "CHANGELOG.md", "docs/FEATURES.md", "docs/GUIDE.md",
             "docs/DEVELOPMENT.md"]
EXAMPLES = ["examples/sequential_lens.json", "examples/nonsequential_slab.json",
            "examples/integrator_rod.json", "examples/folded_mirror.json"]


def files():
    """Documents, examples and schemas: archive member -> source file."""
    listed = {name: ROOT / name for name in DOCUMENTS + EXAMPLES}
    for schema in sorted((ROOT / "schemas").glob("*.schema.json")):
        listed[f"schemas/{schema.name}"] = schema
    return listed


def build_archive(output, name, version, platform, binaries, notices, **details):
    """Write `<output>/<name>.tar.gz` with the binaries (mode 755) and the shared files.

    `binaries` maps archive members to source files. `details` are recorded in package.json.
    """
    hashes = {}
    path = output / f"{name}.tar.gz"
    with tarfile.open(path, "w:gz") as archive:
        def add(member, data, mode=0o644):
            info = tarfile.TarInfo(f"{name}/{member}")
            info.size, info.mode, info.mtime = len(data), mode, 0
            archive.addfile(info, io.BytesIO(data))
            hashes[member] = hashlib.sha256(data).hexdigest()

        for member, source in binaries.items():
            add(member, source.read_bytes(), 0o755)
        for member, source in files().items():
            data = source.read_bytes()
            if member.startswith("examples/") and embedded_document(json.loads(data)):
                raise ValueError("An example contains an embedded original document")
            add(member, data)
        add("THIRD_PARTY_LICENSES.txt", notices)
        package = {"version": version, "license": "MIT", "platform": platform, **details,
                   "sha256": hashes}
        add("package.json", json.dumps(package, indent=2).encode())
    return path

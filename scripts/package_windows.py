"""Build a Windows ZIP from an explicit list of binaries and owned examples."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tomllib
import zipfile
from check_publish import embedded_document

ROOT = Path(__file__).resolve().parents[1]
FILES = {
    "target/release/raytrace-gui.exe": "raytrace-gui.exe",
    "target/release/raytrace-rs.exe": "raytrace-rs.exe",
    "Raytraceを起動.cmd": "Raytraceを起動.cmd",
    "README.md": "README.md",
    "docs/FEATURES.md": "docs/FEATURES.md",
    "docs/GUIDE.md": "docs/GUIDE.md",
    "docs/DEVELOPMENT.md": "docs/DEVELOPMENT.md",
    "LICENSE": "LICENSE",
    "examples/sequential_lens.json": "examples/sequential_lens.json",
    "examples/nonsequential_slab.json": "examples/nonsequential_slab.json",
    "examples/integrator_rod.json": "examples/integrator_rod.json",
    "examples/folded_mirror.json": "examples/folded_mirror.json",
}


def dependency_notices():
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--offline", "--all-features",
        "--filter-platform", "x86_64-pc-windows-msvc", "--format-version", "1",
    ], cwd=ROOT, text=True, encoding="utf-8"))
    active = {node["id"] for node in metadata["resolve"]["nodes"]}
    supplements = tomllib.loads((ROOT / "scripts/third_party_notices.toml").read_text(encoding="utf-8"))["notice"]
    sections = ["Third-party components retain their respective licenses.\n"]
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] not in active or package["name"] == "raytrace-rs":
            continue
        base = Path(package["manifest_path"]).parent
        sections.append(f"\n{'=' * 72}\n{package['name']} {package['version']}\nLicense: {package.get('license')}\nRepository: {package.get('repository')}\n")
        included = set()
        for path in sorted(base.rglob("*")):
            filename = path.name.lower()
            if path.is_file() and (
                "license" in filename or "licence" in filename
                or filename.startswith(("copying", "notice", "ofl.", "ufl."))
                or (package["name"] == "epaint_default_fonts" and filename == "hack-regular.txt")
            ):
                try:
                    content = path.read_text(encoding="utf-8")
                except UnicodeError:
                    continue
                if not content.strip():
                    continue
                sections.append(f"\n--- {path.relative_to(base).as_posix()} ---\n{content}\n")
                included.add(path.relative_to(base).as_posix())
        for notice in supplements:
            if f"{package['name']} {package['version']}" in notice["packages"]:
                sections.append(f"\n--- Supplemental {notice['license']} license ---\nSource: {notice['source']}\n{notice['text']}\n")
                included.add("supplement")
        if not included:
            raise ValueError(f"Missing license text: {package['name']} {package['version']}")
        if package["name"] == "epaint_default_fonts":
            required = {"fonts/Hack-Regular.txt", "fonts/OFL.txt", "fonts/UFL.txt", "fonts/emoji-icon-font-mit-license.txt"}
            if required - included:
                raise ValueError(f"Missing font notices: {sorted(required - included)}")
    return "".join(sections).encode("utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "results/packages")
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "--locked", "--all-features", "--bins"], cwd=ROOT, check=True)
    for binary in ("raytrace-rs", "raytrace-gui"):
        reported = subprocess.check_output([str(ROOT / f"target/release/{binary}.exe"), "--version"], text=True).strip()
        if reported != f"{binary} {version}":
            raise ValueError("Rebuild the binaries for the current package version")
    name = f"raytrace-rs-{version}-windows-x64"
    args.output.mkdir(parents=True, exist_ok=True)
    output = args.output / f"{name}.zip"
    hashes = {}
    notices = dependency_notices()
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for source, destination in FILES.items():
            data = (ROOT / source).read_bytes()
            if source.endswith(".json") and embedded_document(json.loads(data)):
                raise ValueError("An example contains an embedded original document")
            archive.writestr(f"{name}/{destination}", data)
            hashes[destination] = hashlib.sha256(data).hexdigest()
        archive.writestr(f"{name}/package.json", json.dumps({"version": version, "license": "MIT", "platform": "windows-x64", "sha256": hashes}, indent=2))
        archive.writestr(f"{name}/THIRD_PARTY_LICENSES.txt", notices)
        archive.writestr(f"{name}/START_HERE.md", f"""# Raytrace {version}

ZIPを展開し、「Raytraceを起動.cmd」を開きます。基本の光線追跡にRustやOpticStudioは必要ありません。

Visual C++ v14ランタイムを使います。VCRUNTIME140.dllが不足する環境では、[Microsoft公式のx64ランタイム](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)を導入してください。

部品を読み込んで配置し、「計算する」で追跡します。走査は選択部品の「操作 → この部品の値を走査…」から開きます。

`examples` に解析用のJSONサンプルがあります。STEPの初回読込にはFreeCADのPythonが必要です。分割済みメッシュを含むJSONはRustだけで追跡できます。

設定・CADキャッシュ・走査記録は `%LOCALAPPDATA%/Raytrace` に保存します。`RAYTRACE_DATA_DIR` を設定すると保存先を変更できます。

機能と対応範囲は [機能一覧](docs/FEATURES.md)、詳しい操作は [操作ガイド](docs/GUIDE.md) を参照してください。[README](README.md) からも各文書へ進めます。

版と実行ファイル・サンプル等のSHA-256は `package.json` に記載しています。本ソフトはMITライセンスです。本文は `LICENSE`、依存ライブラリとフォントの利用条件は `THIRD_PARTY_LICENSES.txt` に同梱しています。
""")
    with zipfile.ZipFile(output) as archive:
        if archive.testzip() is not None:
            raise ValueError("Package integrity check failed")
        if len(archive.namelist()) != len(FILES) + 3:
            raise ValueError("Unexpected package members")
    print(output.resolve())


if __name__ == "__main__":
    main()

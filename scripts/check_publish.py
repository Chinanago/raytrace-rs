"""Check the Git index before committing/publishing. Reads staged bytes, not local data."""
import json
from pathlib import Path, PurePosixPath
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
FILES = {
    ".gitattributes", ".gitignore", "Cargo.toml", "Cargo.lock", "deny.toml", "README.md", "LICENSE", "CHANGELOG.md",
    "docs/FEATURES.md", "docs/GUIDE.md", "docs/DEVELOPMENT.md", "docs/DESIGN.md", "docs/CAPABILITIES.md",
    "Raytraceを起動.cmd", "examples/exchange_check.rs", "examples/assembly_demo.rs",
    "examples/validation_suite.rs",
    "examples/performance_check.rs",
    "examples/throughput_check.rs",
    "examples/trace_limits.rs",
    "examples/step_check.rs",
    ".github/workflows/windows.yml",
    ".github/workflows/release.yml",
    "scripts/package_linux.py", "scripts/package_macos.py", "scripts/package_mcpb.py",
    "scripts/smoke.py", "scripts/unix_archive.py", "scripts/check_install.py",
    "install.sh", "install.ps1",
    "scripts/check_portable.py", "scripts/subset_fonts.py",
    "scripts/package_windows.py",
    "scripts/third_party_notices.toml",
    "scripts/check_publish.py", "scripts/zemax_exchange_probe.py",
    "scripts/opticstudio_nsc_probe.py", "examples/nsc_reference.rs",
    "scripts/benchmark_compare.py",
    "AGENTS.md", "CLAUDE.md", "clippy.toml", ".claude-plugin/marketplace.json", ".mcp.json", ".cargo/config.toml", "xtask/Cargo.toml",
    "skills/raytrace-rs/SKILL.md", "skills/raytrace-rs/LICENSE",
    "skills/raytrace-rs/agents/openai.yaml", "skills/raytrace-rs/references/workflows.md",
}
DEMOS = {
    "examples/sequential_lens.json", "examples/nonsequential_slab.json",
    "examples/integrator_rod.json", "examples/folded_mirror.json",
}
FIXTURES = {"tests/fixtures/step_legacy_hits.json"}
SCHEMAS = {f"schemas/{name}.schema.json" for name in (
    "request", "response", "scene", "design", "run", "scan_plan", "scan", "convergence_plan", "convergence",
)}
FONT_ASSETS = {"crates/raytrace-gui/assets/fonts/" + name for name in (
    "NotoSansJP-Regular.otf", "NotoSansJP-Bold.otf", "NotoSansMono-Regular.ttf",
    "OFL-NotoSansJP.txt", "OFL-NotoSansMono.txt",
)}
CRATES = {"raytrace-core", "raytrace-formats", "raytrace-doc", "raytrace-io", "raytrace-engine", "raytrace-mcp", "raytrace-cli", "raytrace-gui"}


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


def embedded_document(value):
    if isinstance(value, dict):
        if str(value.get("format", "")).startswith("raytrace-design-") or value.get("zemax") is not None or value.get("zemax_definition") is not None or value.get("provenance") is not None:
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
        source = part.suffix == ".rs" and part.parts[0] in {"src", "tests", "xtask"}
        source = source or (
            len(part.parts) >= 3
            and part.parts[0] == "crates"
            and part.parts[1] in CRATES
            and (part.parts[2:] == ("Cargo.toml",)
                 or (part.parts[2] == "src" and part.suffix == ".rs"))
        )
        if mode != b"100644" or stage != b"0" or not (path in FILES | DEMOS | FIXTURES | SCHEMAS | FONT_ASSETS or source):
            failures.append(f"Unapproved publication path or mode: {path}")
            continue
        content = git("show", f":{path}")
        if path in FONT_ASSETS and part.suffix in {".otf", ".ttf"}:
            signature = b"OTTO" if part.suffix == ".otf" else b"\x00\x01\x00\x00"
            if not content.startswith(signature):
                failures.append(f"Invalid bundled font signature: {path}")
            checked += 1
            continue
        if b"\0" in content:
            failures.append(f"Binary content: {path}")
            continue
        try:
            text = content.decode("utf-8")
            if path in DEMOS | FIXTURES and embedded_document(json.loads(text)):
                failures.append(f"Embedded original project data: {path}")
            if path in FIXTURES:
                fixture = json.loads(text)
                if fixture.get("format") != "owned-step-legacy-hits-v1" or {c["name"] for c in fixture["cases"]} != {"box_mm", "box_inch", "tube", "cone", "sphere_cap", "torus"}:
                    failures.append(f"Unrecognized owned fixture: {path}")
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

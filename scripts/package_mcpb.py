"""Build a Claude Desktop extension (MCPB) that starts `raytrace-rs mcp`.

Usage: package_mcpb.py PLATFORM BINARY OUTPUT_DIR

PLATFORM is `macos` or `windows`. BINARY is the `raytrace-rs` executable for that platform
(a universal binary on macOS). The result is
`OUTPUT_DIR/raytrace-rs-<version>-<platform>.mcpb`, a zip with `manifest.json` at the root and
the executable under `server/`. Spec: https://github.com/modelcontextprotocol/mcpb (MANIFEST.md).
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PLATFORMS = {
    # name -> (compatibility.platforms value, executable file name)
    "macos": ("darwin", "raytrace-rs"),
    "windows": ("win32", "raytrace-rs.exe"),
}
TOOLS = [
    {"name": "list_commands",
     "description": "raytrace-rsの全コマンドのID・種類（query・mutation・job）・要約を一覧する。最初に呼ぶ。"},
    {"name": "describe_command",
     "description": "1つのコマンドの入力と結果のJSONスキーマを返す。初めて使うコマンドの前に呼ぶ。"},
    {"name": "execute",
     "description": "1つのコマンドをセッションで実行する。doc.openで開いた光学系は呼び出しの間も保たれ、doc.setやcomponent.*で編集し、analysis.traceで光線追跡し、session.undoで戻せる。"},
    {"name": "batch",
     "description": "複数のコマンドを同じセッションで順に実行し、応答の一覧を返す。既定では最初の失敗で止まる。"},
]


def workspace_version():
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    return cargo["workspace"]["package"]["version"]


def manifest(platform, version):
    """The extension manifest (MCPB manifest_version 0.3) for one platform."""
    node, executable = PLATFORMS[platform]
    command = f"${{__dirname}}/server/{executable}"
    mcp_config = {"command": command, "args": ["mcp"]}
    if node == "win32":
        # The spec says apps append `.exe` to binary commands on Windows. Say it explicitly
        # in the platform override as well, so the launch does not depend on that rule.
        mcp_config = {"command": "${__dirname}/server/raytrace-rs", "args": ["mcp"],
                      "platform_overrides": {"win32": {"command": command}}}
    return {
        "manifest_version": "0.3",
        "name": "raytrace-rs",
        "display_name": "raytrace-rs",
        "version": version,
        "description": "幾何光学のレイトレーサー。光学系を開いて編集し、光線追跡と光量分布の解析をAIから実行できます。",
        "long_description": (
            "raytrace-rsは幾何光学の光線追跡ソフトウェアです。光源・レンズ・インテグレーターロッド・"
            "スクリーンで照明系や結像系を組み立て、スクリーン上の光量分布と光量収支を求めます。"
            "ZMX形式のレンズデータ、STEP形状、JSONの光学系を読み込めます。\n\n"
            "この拡張は `raytrace-rs mcp` を起動し、Claudeから光学系の読込・編集（取り消し付き）・"
            "光線追跡・解析を行えるようにします。ツールは list_commands・describe_command・"
            "execute・batch の4つで、コマンドは describe_command でスキーマを確認できます。"
            "ファイルの読み書きはClaude Desktopを起動した利用者の権限で行います。\n\n"
            "ベータ版のため、コマンドや保存形式は予告なく変わることがあります。"
        ),
        "author": {"name": "Chinanago", "url": "https://github.com/Chinanago"},
        "repository": {"type": "git", "url": "https://github.com/Chinanago/raytrace-rs.git"},
        "homepage": "https://github.com/Chinanago/raytrace-rs",
        "documentation": "https://github.com/Chinanago/raytrace-rs/blob/main/docs/GUIDE.md",
        "support": "https://github.com/Chinanago/raytrace-rs/issues",
        "license": "MIT",
        "keywords": ["optics", "raytracing", "zmx", "simulation"],
        "server": {"type": "binary", "entry_point": f"server/{executable}", "mcp_config": mcp_config},
        "tools": TOOLS,
        "compatibility": {"platforms": [node]},
    }


def check_server(binary):
    """Start the binary as an MCP server and compare its tools with the manifest."""
    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize",
         "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": {"name": "package_mcpb", "version": "0"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    ]
    result = subprocess.run([str(binary), "mcp"], input="".join(json.dumps(m) + "\n" for m in messages),
                            capture_output=True, text=True, encoding="utf-8", timeout=60)
    answers = {m["id"]: m for m in map(json.loads, result.stdout.splitlines()) if "id" in m}
    served = [tool["name"] for tool in answers[2]["result"]["tools"]]
    declared = [tool["name"] for tool in TOOLS]
    if served != declared:
        raise ValueError(f"The manifest lists {declared} but the server offers {served}")


def build(platform, binary, output, check=False):
    if platform not in PLATFORMS:
        raise ValueError(f"Unknown platform {platform!r}; use one of {sorted(PLATFORMS)}")
    if not binary.is_file():
        raise ValueError(f"No such binary: {binary}")
    version = workspace_version()
    if check:
        check_server(binary)
    document = manifest(platform, version)
    _, executable = PLATFORMS[platform]
    output.mkdir(parents=True, exist_ok=True)
    path = output / f"raytrace-rs-{version}-{platform}.mcpb"
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        def add(name, data, mode):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (0o100000 | mode) << 16
            archive.writestr(info, data)

        add("manifest.json", json.dumps(document, indent=2, ensure_ascii=False) + "\n", 0o644)
        add(f"server/{executable}", binary.read_bytes(), 0o755)
        add("LICENSE", (ROOT / "LICENSE").read_bytes(), 0o644)
    with zipfile.ZipFile(path) as archive:
        if archive.testzip() is not None:
            raise ValueError("MCPB integrity check failed")
        if set(archive.namelist()) != {"manifest.json", f"server/{executable}", "LICENSE"}:
            raise ValueError("Unexpected MCPB members")
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("platform", choices=sorted(PLATFORMS))
    parser.add_argument("binary", type=Path)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--check-server", action="store_true",
                        help="run the binary (it must run on this machine) and compare its MCP tools")
    args = parser.parse_args()
    print(build(args.platform, args.binary.resolve(), args.output_dir.resolve(), args.check_server))
    return 0


if __name__ == "__main__":
    sys.exit(main())

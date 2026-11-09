"""Smoke tests shared by the Linux, macOS and Windows packages.

Everything here runs the packaged `raytrace-rs` binary from an extracted archive:
the version, the four owned examples, a replay, a JSON request stream, a PNG export,
and an MCP session over standard input and output.
"""
import json
from pathlib import Path
import subprocess

EXAMPLES = ["examples/sequential_lens.json", "examples/nonsequential_slab.json",
            "examples/integrator_rod.json", "examples/folded_mirror.json"]
MCP_PROTOCOL = "2025-06-18"


def run(cli, *args, stdin=None, cwd, env=None):
    """Run one CLI call and return its JSON responses; fail on a non-zero exit or `ok: false`."""
    result = subprocess.run([str(cli), *map(str, args)], input=stdin, cwd=cwd, env=env,
                            capture_output=True, text=True, encoding="utf-8",
                            errors="replace", timeout=300)
    responses = [json.loads(line) for line in result.stdout.splitlines()]
    failed = [r for r in responses if not r.get("ok")]
    if result.returncode != 0 or not responses or failed:
        raise RuntimeError(f"{args}: exit {result.returncode}\n{result.stdout}\n{result.stderr}")
    return responses


def check_version(cli, name, version, env=None):
    reported = subprocess.check_output([str(cli), "--version"], text=True, env=env).strip()
    if reported != f"{name} {version}":
        raise ValueError(f"Unexpected version: {reported}")


def mcp_session(cli, root, env=None):
    """Drive `raytrace-rs mcp` through a short real session and assert that every call succeeds.

    The calls share one session: open an example, trace it with a small ray count and read the
    power budget of that calculation without naming the document again.
    """
    def call(identifier, tool, arguments=None):
        return {"jsonrpc": "2.0", "id": identifier, "method": "tools/call",
                "params": {"name": tool, "arguments": arguments or {}}}

    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize",
         "params": {"protocolVersion": MCP_PROTOCOL, "capabilities": {},
                    "clientInfo": {"name": "smoke", "version": "0"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
        call(3, "execute", {"command": "doc.open", "params": {"path": "examples/integrator_rod.json"}}),
        call(4, "execute", {"command": "analysis.trace",
                            "params": {"overrides": {"rays_per_source": 256}, "threads": 2,
                                       "output": "results/mcp_rod.json"}}),
        call(5, "execute", {"command": "analysis.budget"}),
    ]
    process = subprocess.run(
        [str(cli), "mcp"], input="".join(json.dumps(m) + "\n" for m in messages),
        cwd=root, env=env, capture_output=True, text=True, encoding="utf-8",
        errors="replace", timeout=300)
    if process.returncode != 0:
        raise RuntimeError(f"mcp exited with {process.returncode}\n{process.stderr}")
    # Progress notifications have no id; answers are matched by id.
    answers = {}
    for line in process.stdout.splitlines():
        message = json.loads(line)
        if "id" in message:
            answers[message["id"]] = message
    for identifier in range(1, 6):
        if identifier not in answers or "error" in answers[identifier]:
            raise RuntimeError(f"MCP request {identifier} failed: {answers.get(identifier)}")
    tools = {tool["name"] for tool in answers[2]["result"]["tools"]}
    if tools != {"list_commands", "describe_command", "execute", "batch"}:
        raise ValueError(f"Unexpected MCP tools: {sorted(tools)}")
    for identifier in (3, 4, 5):
        result = answers[identifier]["result"]
        envelope = result.get("structuredContent", {})
        if result.get("isError") or not envelope.get("ok"):
            raise RuntimeError(f"MCP tool call {identifier} failed: {result}")
    budget = answers[5]["result"]["structuredContent"]["result"]
    if not budget:
        raise ValueError("analysis.budget returned no result")
    if not (Path(root) / "results/mcp_rod.json").is_file():
        raise ValueError("MCP analysis.trace did not write its record")


def smoke_package(root, name, version, env=None):
    """Exercise an extracted package in `root`. `name` is the CLI binary file name."""
    root = Path(root)
    cli = root / name
    check_version(cli, "raytrace-rs", version, env)
    for example in EXAMPLES:
        stem = Path(example).stem
        run(cli, "analysis.trace", f"model.path={example}", "overrides.rays_per_source:=256",
            "threads:=2", f"output=results/{stem}.json", cwd=root, env=env)
    run(cli, "run.replay", "record.path=results/sequential_lens.json", "output=results/replay.json",
        cwd=root, env=env)
    requests = "\n".join(json.dumps(r) for r in [
        {"command": "app.version"},
        {"command": "doc.inspect", "model": {"path": "examples/integrator_rod.json"}},
        {"command": "run.export_screen", "record": {"path": "results/integrator_rod.json"},
         "output": "results/screen"},
    ])
    run(cli, "run", "-", stdin=requests, cwd=root, env=env)
    if not (root / "results/screen/screen.png").is_file():
        raise ValueError("Screen export did not write a PNG")
    for schema in (root / "schemas").glob("*.schema.json"):
        json.loads(schema.read_text(encoding="utf-8"))
    mcp_session(cli, root, env)
    print(f"Smoke-tested {len(EXAMPLES)} examples, replay, JSON requests and an MCP session")

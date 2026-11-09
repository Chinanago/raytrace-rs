"""Install a package with the install script, use it, then uninstall it.

Usage: check_install.py PACKAGE

PACKAGE is the platform archive made by the package scripts:
`raytrace-rs-<version>-linux-x64.tar.gz` or `-macos.tar.gz` (checked with `install.sh`) or
`raytrace-rs-<version>-windows-x64.zip` (checked with `install.ps1` in every PowerShell found:
`pwsh` and Windows PowerShell 5.1). Everything happens in a temporary home directory
(`HOME`, or `LOCALAPPDATA` on Windows) so the real installation is left alone, except that the
Windows script edits the user PATH in the registry; that value is restored at the end.

The installed `raytrace-rs --version` and the shared MCP session of `smoke.py` run against the
installed copy, and uninstalling must remove everything the installer put on disk.
"""
import argparse
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import zipfile

from smoke import check_version, mcp_session

ROOT = Path(__file__).resolve().parents[1]


def sha256_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_checksum(path, name=None, digest=None):
    """Write `<path>.sha256` in the release format `<hex>  <filename>`."""
    name = name or Path(path).name
    Path(str(path) + ".sha256").write_text(f"{digest or sha256_file(path)}  {name}\n", encoding="utf-8")


def package_version(package):
    """The version recorded in the archive's package.json."""
    if package.suffix == ".zip":
        with zipfile.ZipFile(package) as archive:
            member = next(n for n in archive.namelist() if n.endswith("/package.json"))
            return json.loads(archive.read(member))["version"]
    with tarfile.open(package) as archive:
        member = next(m for m in archive.getmembers() if m.name.endswith("/package.json"))
        return json.loads(archive.extractfile(member).read())["version"]


def run(command, env, expect_ok=True, **kwargs):
    result = subprocess.run(command, env=env, capture_output=True, text=True, encoding="utf-8",
                            errors="replace", timeout=600, **kwargs)
    if expect_ok and result.returncode != 0:
        raise RuntimeError(f"{command} exited with {result.returncode}\n{result.stdout}\n{result.stderr}")
    if not expect_ok and result.returncode == 0:
        raise RuntimeError(f"{command} should have failed\n{result.stdout}\n{result.stderr}")
    return result


def expect(condition, message):
    if not condition:
        raise ValueError(message)


def expect_in(needles, text, label):
    for needle in needles:
        expect(needle in text, f"{label} does not mention {needle!r}:\n{text}")


# ---------------------------------------------------------------- macOS and Linux

class Releases:
    """A local HTTP server laid out like github.com/<repo>/releases, for the download path."""

    def __init__(self, directory):
        class Quiet(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *args):
                pass

        handler = functools.partial(Quiet, directory=str(directory))
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server.server_port}"

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *exc):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def unix_environment(home):
    environment = {k: v for k, v in os.environ.items() if not k.startswith("RAYTRACE_")}
    environment["HOME"] = str(home)
    for name in ("NO_PROXY", "no_proxy"):
        environment[name] = "127.0.0.1,localhost"
    return environment


def check_unix(package, version):
    platform = "macos" if "-macos" in package.name else "linux-x64"
    script = ROOT / "install.sh"
    steps = []
    with tempfile.TemporaryDirectory(prefix="install_") as temporary:
        temp = Path(temporary)
        home = temp / "home"
        home.mkdir()
        environment = unix_environment(home)
        binary_dir = home / ".local/bin"
        share = home / ".local/share/raytrace-rs"
        cli = binary_dir / "raytrace-rs"
        install = ["sh", str(script)]
        from_package = {**environment, "RAYTRACE_INSTALL_FROM": str(package)}

        expect_in(["--uninstall", "RAYTRACE_INSTALL_DIR"], run([*install, "--help"], environment).stdout, "--help")
        run([*install, "--bogus"], environment, expect_ok=False)
        steps.append("install.sh --help and a bad option")

        # A share directory the installer did not make is never replaced.
        foreign = temp / "foreign"
        foreign.mkdir()
        (foreign / "mine.txt").write_text("keep", encoding="utf-8")
        result = run(install, {**from_package, "RAYTRACE_SHARE_DIR": str(foreign)}, expect_ok=False)
        expect((foreign / "mine.txt").is_file() and not cli.exists(), "A foreign share directory was touched")
        steps.append("Refuses to replace a folder it did not create")

        # Install from the local archive (sh = dash on Debian, bash 3.2 on macOS).
        result = run(install, from_package)
        output = result.stdout
        expect(cli.is_file() and os.access(cli, os.X_OK), "raytrace-rs was not installed")
        expect((share / "examples/integrator_rod.json").is_file(), "Examples were not installed")
        expect((share / "schemas/request.schema.json").is_file(), "Schemas were not installed")
        expect((share / "docs/GUIDE.md").is_file(), "Docs were not installed")
        expect(not (share / "raytrace-rs").exists(), "A binary was copied into the share directory")
        if platform == "macos":
            expect((binary_dir / "raytrace-gui").is_file(), "raytrace-gui was not installed")
        else:
            expect(not (binary_dir / "raytrace-gui").exists(), "Unexpected raytrace-gui")
        expect_in([f"claude mcp add raytrace-rs -- {cli} mcp", "/plugin install raytrace-rs --marketplace Chinanago/raytrace-rs",
                   "[mcp_servers.raytrace-rs]", f'command = "{cli}"', 'args = ["mcp"]',
                   "export PATH=", f"raytrace-rs {version} is ready"], output, "The install output")
        if platform == "macos":
            expect_in(["claude_desktop_config.json", f'"command": "{cli}"', "raytrace-rs-macos.mcpb"], output, "The install output")
        steps.append("Install from a local archive; setup text with the absolute path")

        check_version(cli, "raytrace-rs", version, environment)
        mcp_session(cli, share, environment)
        steps.append("Installed --version and MCP session (initialize, doc.open, analysis.trace, analysis.budget)")

        # Updating replaces the previous share copy.
        (share / "stale.txt").write_text("old", encoding="utf-8")
        run(install, from_package)
        expect(not (share / "stale.txt").exists(), "The previous share copy was not replaced")
        check_version(cli, "raytrace-rs", version, environment)
        steps.append("Reinstall replaces the previous copy")

        # A custom location, with a space, shows up quoted in the claude command.
        custom = temp / "my tools"
        custom_share = temp / "my share"
        result = run(install, {**from_package, "RAYTRACE_INSTALL_DIR": str(custom), "RAYTRACE_SHARE_DIR": str(custom_share)})
        expect((custom / "raytrace-rs").is_file() and (custom_share / "examples").is_dir(), "Custom locations were ignored")
        expect_in([f'-- "{custom / "raytrace-rs"}" mcp'], result.stdout, "The install output")
        run([*install, "--uninstall"], {**environment, "RAYTRACE_INSTALL_DIR": str(custom), "RAYTRACE_SHARE_DIR": str(custom_share)})
        expect(not (custom / "raytrace-rs").exists() and not custom_share.exists(), "Custom uninstall left files")
        steps.append("RAYTRACE_INSTALL_DIR and RAYTRACE_SHARE_DIR")

        # Download path: fixed and versioned names, and the checksum.
        releases = temp / "releases"
        fixed = releases / "latest/download"
        tagged = releases / f"download/v{version}"
        fixed.mkdir(parents=True)
        tagged.mkdir(parents=True)
        fixed_name = f"raytrace-rs-{platform}.tar.gz"
        versioned_name = f"raytrace-rs-{version}-{platform}.tar.gz"
        shutil.copy(package, fixed / fixed_name)
        shutil.copy(package, tagged / versioned_name)
        write_checksum(fixed / fixed_name)
        write_checksum(tagged / versioned_name)
        run([*install, "--uninstall"], environment)
        expect(not cli.exists() and not share.exists(), "Uninstall left files")
        with Releases(releases) as server:
            served = {**environment, "RAYTRACE_RELEASES_URL": server.url}
            result = run(install, served)
            expect_in(["Checksum verified", f"/latest/download/{fixed_name}"], result.stdout, "The download output")
            check_version(cli, "raytrace-rs", version, environment)
            run([*install, "--uninstall"], environment)
            result = run(install, {**served, "RAYTRACE_VERSION": f"v{version}"})
            expect_in(["Checksum verified", f"/download/v{version}/{versioned_name}"], result.stdout, "The tagged download output")
            run([*install, "--uninstall"], environment)

            # A corrupted checksum stops the install before anything is written.
            write_checksum(fixed / fixed_name, digest="0" * 64)
            result = run(install, served, expect_ok=False)
            expect("checksum mismatch" in result.stderr, f"Expected a checksum error:\n{result.stderr}")
            expect(not cli.exists() and not share.exists(), "A corrupt download was installed")

            # A missing checksum file only warns.
            (fixed / (fixed_name + ".sha256")).unlink()
            result = run(install, served)
            expect("could not be downloaded" in result.stderr, "Expected a warning for the missing checksum")
            run([*install, "--uninstall"], environment)

            # A missing release is an error.
            run(install, {**served, "RAYTRACE_VERSION": "v0.0.1"}, expect_ok=False)
        steps.append("Download by fixed name and by tag, sha256 verified, mismatch rejected")

        # Uninstall removes everything it installed.
        run(install, from_package)
        result = run([*install, "--uninstall"], environment)
        expect_in(["claude mcp remove raytrace-rs"], result.stdout, "The uninstall output")
        expect(not cli.exists() and not (binary_dir / "raytrace-gui").exists() and not share.exists(),
               "Uninstall left files behind")
        leftovers = [p for p in home.rglob("*") if p.is_file()]
        expect(not leftovers, f"Files left in the home directory: {leftovers}")
        run([*install, "--uninstall"], environment)
        steps.append("Uninstall leaves no files; a second uninstall is harmless")
    return steps


# ---------------------------------------------------------------------- Windows

def powershells():
    """The PowerShell hosts to test: PowerShell 7 and Windows PowerShell 5.1 when present."""
    found = []
    for name in ("pwsh", "powershell"):
        path = shutil.which(name)
        if path:
            found.append(path)
    return found


def user_path():
    """The user PATH value and its registry type, or (None, None) when it is not set."""
    import winreg
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment") as key:
        try:
            return winreg.QueryValueEx(key, "Path")
        except FileNotFoundError:
            return None, None


def restore_user_path(value, kind):
    import winreg
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment", 0, winreg.KEY_SET_VALUE) as key:
        if value is None:
            try:
                winreg.DeleteValue(key, "Path")
            except FileNotFoundError:
                pass
        else:
            winreg.SetValueEx(key, "Path", 0, kind, value)


def check_windows_host(host, package, version, temp):
    script = ROOT / "install.ps1"
    local = temp / "localappdata"
    local.mkdir()
    environment = {k: v for k, v in os.environ.items() if not k.startswith("RAYTRACE_")}
    environment["LOCALAPPDATA"] = str(local)
    folder = local / "Programs" / "raytrace-rs"
    exe = folder / "raytrace-rs.exe"
    # -ExecutionPolicy Bypass: the default policy of a Windows client refuses downloaded scripts.
    install = [host, "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(script)]

    def on_user_path():
        value, _ = user_path()
        entries = [e.rstrip("\\").lower() for e in (value or "").split(";") if e]
        return str(folder).rstrip("\\").lower() in entries

    steps = []
    # A corrupted checksum beside the zip stops the install.
    broken = temp / package.name
    shutil.copy(package, broken)
    write_checksum(broken, name=package.name, digest="0" * 64)
    result = run([*install, "-From", str(broken)], environment, expect_ok=False)
    expect(not folder.exists() and not on_user_path(), "A corrupt package was installed")
    steps.append("checksum mismatch rejected")

    write_checksum(broken, name=package.name)
    result = run([*install, "-From", str(broken)], environment)
    # Write-Host reaches stdout in both hosts, but warnings go to stderr; read both.
    output = result.stdout + result.stderr
    expect(exe.is_file(), "raytrace-rs.exe was not installed")
    expect((folder / "examples/integrator_rod.json").is_file() and (folder / "schemas/request.schema.json").is_file()
           and (folder / "docs/GUIDE.md").is_file(), "Examples, schemas or docs are missing")
    expect(on_user_path(), "The install folder was not added to the user PATH")
    escaped = str(exe).replace("\\", "\\\\")
    expect_in([f"raytrace-rs {version}", "claude mcp add raytrace-rs --", str(exe), "claude_desktop_config.json",
               f'"command": "{escaped}"', "[mcp_servers.raytrace-rs]", "args = [\"mcp\"]",
               "/plugin install raytrace-rs --marketplace Chinanago/raytrace-rs"], output, "The install output")
    steps.append("install -From, checksum verified, user PATH, setup text")

    check_version(exe, "raytrace-rs", version, environment)
    mcp_session(exe, folder, environment)
    steps.append("installed --version and MCP session")

    (folder / "stale.txt").write_text("old", encoding="utf-8")
    run([*install, "-From", str(broken)], environment)
    expect(not (folder / "stale.txt").exists() and exe.is_file(), "The previous copy was not replaced")
    path_entries = [e for e in (user_path()[0] or "").split(";") if e.rstrip("\\").lower() == str(folder).lower()]
    expect(len(path_entries) == 1, f"The PATH entry was duplicated: {path_entries}")
    steps.append("reinstall replaces the copy without duplicating the PATH entry")

    run([*install, "-Uninstall"], environment)
    expect(not folder.exists() and not on_user_path(), "Uninstall left the folder or the PATH entry")
    run([*install, "-Uninstall"], environment)
    steps.append("uninstall removes the folder and the PATH entry; a second one is harmless")
    return steps


def check_windows(package, version):
    if os.name != "nt":
        raise ValueError("A .zip package is checked with install.ps1 on Windows only")
    hosts = powershells()
    if not hosts:
        raise ValueError("No PowerShell (pwsh or powershell) was found")
    original = user_path()
    steps = []
    try:
        for host in hosts:
            with tempfile.TemporaryDirectory(prefix="install_") as temporary:
                for step in check_windows_host(host, package, version, Path(temporary)):
                    steps.append(f"{Path(host).stem}: {step}")
    finally:
        restore_user_path(*original)
    return steps


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("package", type=Path)
    args = parser.parse_args()
    package = args.package.resolve()
    if not package.is_file():
        raise ValueError(f"No such package: {package}")
    version = package_version(package)
    steps = check_windows(package, version) if package.suffix == ".zip" else check_unix(package, version)
    for step in steps:
        print(f"ok  {step}")
    print(f"Install checks passed for {package.name}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError) as error:
        print(error, file=sys.stderr)
        sys.exit(1)

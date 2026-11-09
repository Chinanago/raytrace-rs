#!/bin/sh
# raytrace-rs installer for macOS and Linux (POSIX sh, no sudo).
#
#   curl -fsSL https://raw.githubusercontent.com/Chinanago/raytrace-rs/main/install.sh | sh
#
# Messages are ASCII English on purpose: they must read correctly in any terminal.
# The Japanese guide is docs/AI.md.
set -eu

REPO="Chinanago/raytrace-rs"
RELEASES_URL="${RAYTRACE_RELEASES_URL:-https://github.com/$REPO/releases}"
INSTALL_DIR="${RAYTRACE_INSTALL_DIR:-${HOME:-}/.local/bin}"
SHARE_DIR="${RAYTRACE_SHARE_DIR:-${HOME:-}/.local/share/raytrace-rs}"
VERSION="${RAYTRACE_VERSION:-}"
FROM="${RAYTRACE_INSTALL_FROM:-}"

usage() {
    cat <<'EOF'
Install raytrace-rs (CLI, MCP server and, on macOS, the GUI) into your home directory.

Usage: install.sh [--uninstall] [--help]

Environment variables:
  RAYTRACE_VERSION       Release tag to install, for example v0.12.0 (default: the latest release)
  RAYTRACE_INSTALL_DIR   Where the binaries go (default: $HOME/.local/bin)
  RAYTRACE_SHARE_DIR     Where examples, schemas and docs go (default: $HOME/.local/share/raytrace-rs)
  RAYTRACE_INSTALL_FROM  Install from this local .tar.gz instead of downloading it
  RAYTRACE_RELEASES_URL  Base URL of the releases (default: https://github.com/Chinanago/raytrace-rs/releases)

The download is verified against the .sha256 file next to it when that file exists.
No shell startup file is edited and no sudo is used.
EOF
}

say() { printf '%s\n' "$*"; }
note() { printf '%s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

TMP=""
cleanup() {
    if [ -n "$TMP" ]; then
        rm -rf "$TMP"
    fi
}
trap cleanup EXIT
trap 'exit 1' INT TERM HUP

# Print the sha256 of a file.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$1" | awk '{print $NF}'
    else
        return 1
    fi
}

# Compare the file with the first field of a checksum file: verify FILE CHECKSUM_FILE.
verify() {
    expected=$(awk 'NR == 1 {print tolower($1)}' "$2")
    if [ -z "$expected" ]; then
        die "the checksum file is empty: $2"
    fi
    if ! actual=$(sha256_of "$1"); then
        note "warning: no sha256 tool found (sha256sum, shasum, openssl); skipping verification"
        return 0
    fi
    if [ "$actual" != "$expected" ]; then
        die "checksum mismatch for $(basename "$1"): expected $expected, got $actual. Nothing was installed."
    fi
    say "Checksum verified (sha256 $actual)"
}

# Download URL to FILE with curl or wget.
download() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        die "curl or wget is required"
    fi
}

# Quote a path for a shell command line only when it needs it.
shell_quote() {
    case "$1" in
        *[!A-Za-z0-9_./:@%+=,-]*) printf '"%s"' "$1" ;;
        *) printf '%s' "$1" ;;
    esac
}

# Escape a string for a JSON or TOML basic string.
escape_string() {
    printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

# True when DIR is a share directory made by this installer (package.json from the archive).
is_share_dir() {
    [ -f "$1/package.json" ] && [ -d "$1/examples" ]
}

# Refuse paths that would be dangerous to replace or delete.
check_share_dir() {
    case "$SHARE_DIR" in
        ""|/|"${HOME:-/}"|"${HOME:-/}/") die "unsafe RAYTRACE_SHARE_DIR: '$SHARE_DIR'" ;;
    esac
}

uninstall() {
    check_share_dir
    removed=0
    for name in raytrace-rs raytrace-gui; do
        if [ -e "$INSTALL_DIR/$name" ]; then
            rm -f "$INSTALL_DIR/$name"
            say "Removed $INSTALL_DIR/$name"
            removed=1
        fi
    done
    if is_share_dir "$SHARE_DIR"; then
        rm -rf "$SHARE_DIR"
        say "Removed $SHARE_DIR"
        removed=1
    elif [ -e "$SHARE_DIR" ]; then
        note "warning: $SHARE_DIR was not created by this installer; it was left alone"
    fi
    if [ "$removed" = 0 ]; then
        say "Nothing to remove."
    fi
    say ""
    say "If you registered the MCP server, remove it too:"
    say "  Claude Code:    claude mcp remove raytrace-rs"
    say "  Claude Desktop: delete the raytrace-rs entry from claude_desktop_config.json"
    say "  Codex:          delete the [mcp_servers.raytrace-rs] section from ~/.codex/config.toml"
    say "Settings and caches in the data folder are kept (see docs/GUIDE.md)."
}

# Sets ASSET (the file to download) and URL for this platform.
choose_asset() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Darwin) platform="macos" ;;
        Linux)
            case "$arch" in
                x86_64|amd64) platform="linux-x64" ;;
                *) die "unsupported CPU '$arch' on Linux; only x86_64 is built. Build from source: cargo install --git https://github.com/$REPO --locked raytrace-cli" ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*) die "this is the macOS/Linux installer. On Windows run install.ps1 in PowerShell." ;;
        *) die "unsupported OS '$os'" ;;
    esac
    if [ -n "$VERSION" ]; then
        tag="v${VERSION#v}"
        ASSET="raytrace-rs-${tag#v}-$platform.tar.gz"
        URL="$RELEASES_URL/download/$tag/$ASSET"
    else
        ASSET="raytrace-rs-$platform.tar.gz"
        URL="$RELEASES_URL/latest/download/$ASSET"
    fi
}

install_dir_absolute() {
    mkdir -p "$INSTALL_DIR"
    INSTALL_DIR=$(cd "$INSTALL_DIR" && pwd)
}

install_binary() {
    # Copy next to the target and rename, so a running copy is replaced atomically.
    cp "$1" "$INSTALL_DIR/$2.new.$$"
    chmod 755 "$INSTALL_DIR/$2.new.$$"
    mv -f "$INSTALL_DIR/$2.new.$$" "$INSTALL_DIR/$2"
}

print_setup() {
    exe="$INSTALL_DIR/raytrace-rs"
    quoted=$(shell_quote "$exe")
    escaped=$(escape_string "$exe")
    say ""
    say "Set up your AI client (copy and paste):"
    say ""
    say "Claude Code"
    say "  claude mcp add raytrace-rs -- $quoted mcp"
    say "  (add --scope user after 'add' to use it in every project)"
    say "  or, with the plugin (skill and MCP server; needs raytrace-rs on PATH):"
    say "  /plugin install raytrace-rs --marketplace Chinanago/raytrace-rs"
    say "  Check with /mcp inside Claude Code."
    if [ "$(uname -s)" = "Darwin" ]; then
        say ""
        say "Claude Desktop"
        say "  Recommended: open raytrace-rs-macos.mcpb from"
        say "    $RELEASES_URL/latest/download/raytrace-rs-macos.mcpb"
        say "  Or add this to ~/Library/Application Support/Claude/claude_desktop_config.json"
        say "  (Claude Desktop does not read your shell PATH, so the full path is needed),"
        say "  then quit and restart Claude Desktop:"
        say ""
        say "  {"
        say "    \"mcpServers\": {"
        say "      \"raytrace-rs\": {"
        say "        \"command\": \"$escaped\","
        say "        \"args\": [\"mcp\"]"
        say "      }"
        say "    }"
        say "  }"
    fi
    say ""
    say "Codex: add this to ~/.codex/config.toml"
    say ""
    say "  [mcp_servers.raytrace-rs]"
    say "  command = \"$escaped\""
    say "  args = [\"mcp\"]"
    say ""
    say "Examples, schemas and docs: $SHARE_DIR"
    say "Guide (Japanese): $SHARE_DIR/docs/AI.md"
}

print_path_hint() {
    case ":${PATH:-}:" in
        *":$INSTALL_DIR:"*) return 0 ;;
    esac
    case "$INSTALL_DIR" in
        "$HOME"/*)
            relative=${INSTALL_DIR#"$HOME"/}
            line="export PATH=\"\$HOME/$relative:\$PATH\""
            ;;
        *) line="export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
    esac
    say ""
    say "$INSTALL_DIR is not on your PATH. To run raytrace-rs by name, add it (then open a new terminal):"
    say "  zsh:  echo '$line' >> ~/.zshrc"
    say "  bash: echo '$line' >> ~/.bashrc"
    say "AI clients below use the full path, so this is optional for them."
}

install() {
    if [ -z "${HOME:-}" ]; then
        die "HOME is not set"
    fi
    check_share_dir
    if [ -e "$SHARE_DIR" ] && ! is_share_dir "$SHARE_DIR"; then
        die "$SHARE_DIR exists but was not created by this installer. Remove it or set RAYTRACE_SHARE_DIR to another folder."
    fi
    TMP=$(mktemp -d "${TMPDIR:-/tmp}/raytrace-install.XXXXXX")

    if [ -n "$FROM" ]; then
        [ -f "$FROM" ] || die "no such file: $FROM"
        archive="$FROM"
        if [ -f "$FROM.sha256" ]; then
            verify "$archive" "$FROM.sha256"
        fi
    else
        choose_asset
        archive="$TMP/$ASSET"
        say "Downloading $URL"
        download "$URL" "$archive" || die "download failed: $URL"
        if download "$URL.sha256" "$TMP/$ASSET.sha256" 2>/dev/null; then
            verify "$archive" "$TMP/$ASSET.sha256"
        else
            note "warning: $ASSET.sha256 could not be downloaded; the file was not verified"
        fi
    fi

    mkdir "$TMP/extract"
    tar xzf "$archive" -C "$TMP/extract" || die "could not extract $archive"
    root=""
    for entry in "$TMP/extract"/*; do
        if [ -d "$entry" ] && [ -f "$entry/raytrace-rs" ]; then
            root="$entry"
        fi
    done
    [ -n "$root" ] || die "the archive does not contain raytrace-rs"

    install_dir_absolute
    for name in raytrace-rs raytrace-gui; do
        if [ -f "$root/$name" ]; then
            install_binary "$root/$name" "$name"
            say "Installed $INSTALL_DIR/$name"
        fi
    done

    # Everything except the binaries is shared data; replace the previous copy.
    for name in raytrace-rs raytrace-gui; do
        rm -f "$root/$name"
    done
    mkdir -p "$(dirname "$SHARE_DIR")"
    rm -rf "$SHARE_DIR"
    mv "$root" "$SHARE_DIR"
    say "Installed examples, schemas and docs in $SHARE_DIR"

    if [ "$(uname -s)" = "Darwin" ] && command -v xattr >/dev/null 2>&1; then
        # The binaries are not notarized. Clearing quarantine is harmless when it is not set.
        for name in raytrace-rs raytrace-gui; do
            if [ -e "$INSTALL_DIR/$name" ]; then
                xattr -dr com.apple.quarantine "$INSTALL_DIR/$name" 2>/dev/null || true
            fi
        done
        xattr -dr com.apple.quarantine "$SHARE_DIR" 2>/dev/null || true
    fi

    if ! version=$("$INSTALL_DIR/raytrace-rs" --version 2>&1); then
        die "the installed raytrace-rs does not start: $version"
    fi
    say ""
    say "$version is ready."

    print_path_hint
    print_setup
}

action="install"
for arg in "$@"; do
    case "$arg" in
        --uninstall) action="uninstall" ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "unknown option: $arg" ;;
    esac
done

if [ "$action" = "uninstall" ]; then
    if [ -z "${HOME:-}" ]; then
        die "HOME is not set"
    fi
    uninstall
else
    install
fi

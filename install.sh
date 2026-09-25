#!/usr/bin/env bash
# Usage: curl -fsSL https://raw.githubusercontent.com/ntntlang/ntnt/main/install.sh | bash -s -- --version v0.5.4
# No source fallback: a requested release must never silently install main.
set -euo pipefail

REPO="ntntlang/ntnt"
INSTALL_DIR="$HOME/.local/bin"
VERSION="${NTNT_VERSION:-}"
STARTER_KIT=true
TMP_DIR=""
STAGED_BINARY=""
fail() { printf 'NTNT installer: %s\n' "$*" >&2; exit 1; }
cleanup() {
    [ -z "$TMP_DIR" ] || rm -rf "$TMP_DIR"
    [ -z "$STAGED_BINARY" ] || rm -f "$STAGED_BINARY"
}
trap cleanup EXIT

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] && [ -n "$2" ] || fail '--version requires a release version'
            VERSION="$2"; shift 2 ;;
        --no-starter-kit) STARTER_KIT=false; shift ;;
        --help|-h)
            printf 'Usage: bash install.sh [--version vX.Y.Z] [--no-starter-kit]\n'
            printf 'NTNT_VERSION also selects a release; --version takes precedence. Default: latest.\n'
            exit 0 ;;
        *) fail "Unknown argument: $1" ;;
    esac
done

check_command() { command -v "$1" >/dev/null 2>&1; }
download_file() {
    if check_command curl; then
        curl -fsSL --proto '=https' --tlsv1.2 --max-time 120 "$1" -o "$2"
    elif check_command wget; then
        wget -q --https-only --timeout=120 "$1" -O "$2"
    else
        fail 'curl or wget is required'
    fi
}

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) PLATFORM=macos-arm64 ;;
    Linux-x86_64) PLATFORM=linux-x64 ;;
    Linux-armv7l) PLATFORM=linux-armv7 ;;
    *) fail "No release binary for $(uname -s)-$(uname -m). Build the desired tag manually; no source fallback was attempted." ;;
esac
TMP_DIR=$(mktemp -d)
if [ -z "$VERSION" ]; then
    download_file "https://api.github.com/repos/$REPO/releases/latest" "$TMP_DIR/latest.json" || fail 'Cannot resolve latest release; use --version'
    VERSION=$(sed -nE 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' "$TMP_DIR/latest.json")
    [ -n "$VERSION" ] || fail 'Latest release response has no tag_name'
fi
VERSION="v${VERSION#v}"
[[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$ ]] || fail "Invalid release version: $VERSION"
ARCHIVE="ntnt-$PLATFORM.tar.gz"
URL="https://github.com/$REPO/releases/download/$VERSION/$ARCHIVE"
printf 'Installing NTNT %s for %s\n' "$VERSION" "$PLATFORM"
download_file "$URL" "$TMP_DIR/$ARCHIVE" || fail "Release binary unavailable: $URL"
download_file "$URL.sha256" "$TMP_DIR/$ARCHIVE.sha256" || fail 'Release checksum unavailable'

# Parse one checksum for this exact archive. Never follow paths in a checksum file.
CHECKSUM=$(tr -d '\r' < "$TMP_DIR/$ARCHIVE.sha256")
[[ "$CHECKSUM" =~ ^([0-9a-fA-F]{64})[[:blank:]]+\*?([^[:space:]]+)$ ]] || fail 'Malformed release checksum'
EXPECTED="${BASH_REMATCH[1]}"
[ "${BASH_REMATCH[2]}" = "$ARCHIVE" ] || fail 'Checksum names a different archive'
if check_command sha256sum; then
    ACTUAL=$(sha256sum "$TMP_DIR/$ARCHIVE")
elif check_command shasum; then
    ACTUAL=$(shasum -a 256 "$TMP_DIR/$ARCHIVE")
else
    fail 'sha256sum or shasum is required'
fi
ACTUAL="${ACTUAL%% *}"
BINARY_HASH_TOOL=sha256sum
check_command sha256sum || BINARY_HASH_TOOL=shasum
hash_binary() {
    local digest
    if [ "$BINARY_HASH_TOOL" = sha256sum ]; then
        digest=$(sha256sum "$1") || return 1
    else
        digest=$(shasum -a 256 "$1") || return 1
    fi
    printf '%s\n' "${digest%% *}"
}
[ "$(printf '%s' "$ACTUAL" | tr 'A-F' 'a-f')" = "$(printf '%s' "$EXPECTED" | tr 'A-F' 'a-f')" ] || fail 'SHA256 mismatch; nothing installed'

# Accept the binary and the optional static OpenSSL license, nothing else.
# Extract bytes, not archive-controlled paths (including archive symlinks).
CONTENTS=$(tar -tzf "$TMP_DIR/$ARCHIVE" | LC_ALL=C sort)
[[ "$CONTENTS" = ntnt || "$CONTENTS" = $'LICENSE.openssl\nntnt' ]] || fail 'Unexpected archive contents'
tar -xOzf "$TMP_DIR/$ARCHIVE" ntnt > "$TMP_DIR/ntnt"
chmod 755 "$TMP_DIR/ntnt"
BINARY_VERSION=$("$TMP_DIR/ntnt" --version) || fail 'Binary cannot run on this system; existing installation unchanged'
[ "$BINARY_VERSION" = "ntnt ${VERSION#v}" ] || fail "Binary version mismatch: $BINARY_VERSION"

# Stage on the destination filesystem, then replace atomically after validation.
[ ! -d "$INSTALL_DIR/ntnt" ] || fail "Install destination is a directory: $INSTALL_DIR/ntnt"
mkdir -p "$INSTALL_DIR"
EXPECTED_BINARY_HASH=$(hash_binary "$TMP_DIR/ntnt")
STAGED_BINARY=$(mktemp "$INSTALL_DIR/.ntnt.XXXXXXXX")
cp "$TMP_DIR/ntnt" "$STAGED_BINARY"
chmod 755 "$STAGED_BINARY"
if [[ "$CONTENTS" = $'LICENSE.openssl\nntnt' ]]; then
    tar -xOzf "$TMP_DIR/$ARCHIVE" LICENSE.openssl > "$TMP_DIR/LICENSE.openssl"
    mkdir -p "$HOME/.local/share/licenses/ntnt"
    cp "$TMP_DIR/LICENSE.openssl" "$HOME/.local/share/licenses/ntnt/LICENSE.openssl"
fi
[ ! -d "$INSTALL_DIR/ntnt" ] || fail "Install destination is a directory: $INSTALL_DIR/ntnt"
mv -f "$STAGED_BINARY" "$INSTALL_DIR/ntnt"
STAGED_BINARY=""
[ "$(hash_binary "$INSTALL_DIR/ntnt")" = "$EXPECTED_BINARY_HASH" ] || fail 'Installed file checksum verification failed'
INSTALLED_VERSION=$("$INSTALL_DIR/ntnt" --version) || fail 'Installed executable verification failed'
[ "$INSTALLED_VERSION" = "$BINARY_VERSION" ] || fail 'Installed executable version mismatch'
printf 'Installed %s to %s/ntnt\n' "$BINARY_VERSION" "$INSTALL_DIR"
printf 'Add to your shell PATH if needed: export PATH="$HOME/.local/bin:$PATH"\n'
printf 'For shell completion: ntnt completions bash (or zsh/fish)\n'

copy_starter_files() {
    mkdir ./ntnt || return 1
    for item in docs examples CLAUDE.md .github; do
        if [ -e "$SOURCE/$item" ]; then
            cp -R "$SOURCE/$item" ./ntnt/ || return 1
        fi
    done
    if [ -d "$SOURCE/.claude/skills" ]; then
        mkdir -p ./ntnt/.claude || return 1
        cp -R "$SOURCE/.claude/skills" ./ntnt/.claude/ || return 1
    fi
}

# Optional convenience files come from the SAME tag, never main. Do not overwrite
# an existing project or clone. Starter-kit failure does not undo a valid binary.
if "$STARTER_KIT"; then
    if [ -e ./ntnt ]; then
        printf 'Skipping starter kit: ./ntnt already exists.\n'
    elif download_file "https://github.com/$REPO/archive/refs/tags/$VERSION.tar.gz" "$TMP_DIR/source.tar.gz" &&
         tar -xzf "$TMP_DIR/source.tar.gz" -C "$TMP_DIR"; then
        SOURCE="$TMP_DIR/ntnt-${VERSION#v}"
        if [ -d "$SOURCE/docs" ]; then
            if copy_starter_files; then
                printf 'Starter kit for %s saved to ./ntnt/\n' "$VERSION"
            else
                printf 'Starter kit incomplete; binary installation succeeded. Check ./ntnt before retrying.\n' >&2
            fi
        else
            printf 'Starter kit unavailable; binary installation succeeded.\n' >&2
        fi
    else
        printf 'Starter kit download failed; binary installation succeeded.\n' >&2
    fi
fi
printf 'Documentation: https://github.com/%s/tree/%s/docs\n' "$REPO" "$VERSION"

#!/bin/bash
# NTNT Language Installer
# Usage: curl -sSf https://raw.githubusercontent.com/ntntlang/ntnt/main/install.sh | bash

set -e

YELLOW='\033[1;33m'
GREEN='\033[0;32m'
RED='\033[0;31m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

REPO="ntntlang/ntnt"
INSTALL_DIR="$HOME/.local/bin"

echo ""
echo -e "${CYAN}============================================${NC}"
echo -e "${CYAN}  NTNT Language Installer${NC}"
echo -e "${CYAN}============================================${NC}"
echo ""

# Check for required tools
check_command() {
    if ! command -v "$1" &> /dev/null; then
        return 1
    fi
    return 0
}

# Detect platform
detect_platform() {
    local os=$(uname -s)
    local arch=$(uname -m)

    case "$os-$arch" in
        Darwin-arm64)  echo "macos-arm64" ;;
        Linux-x86_64)  echo "linux-x64" ;;
        *)             echo "" ;;  # Other platforms build from source
    esac
}

# Get latest release version from GitHub
get_latest_version() {
    local url="https://api.github.com/repos/$REPO/releases/latest"
    local response

    if check_command curl; then
        response=$(curl -sL --max-time 10 "$url" 2>/dev/null) || return 1
    elif check_command wget; then
        response=$(wget -qO- --timeout=10 "$url" 2>/dev/null) || return 1
    else
        return 1
    fi

    # Extract tag_name using grep/sed (portable)
    echo "$response" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' | head -1
}

# Download a file
download_file() {
    local url="$1"
    local output="$2"

    if check_command curl; then
        curl -fsSL --max-time 120 "$url" -o "$output" 2>/dev/null
    elif check_command wget; then
        wget -q --timeout=120 "$url" -O "$output" 2>/dev/null
    else
        echo -e "${RED}Neither curl nor wget found. Cannot download files.${NC}"
        return 1
    fi
}

# Try to download pre-built binary
try_download_binary() {
    local platform=$(detect_platform)

    if [ -z "$platform" ]; then
        echo -e "${YELLOW}No pre-built binary available for this platform ($(uname -s)-$(uname -m)).${NC}"
        return 1
    fi

    echo "Detected platform: $platform"

    local version=$(get_latest_version)
    if [ -z "$version" ]; then
        echo -e "${YELLOW}Could not determine latest version (no releases yet or API unavailable).${NC}"
        return 1
    fi

    echo "Latest version: $version"

    local url="https://github.com/$REPO/releases/download/$version/ntnt-$platform.tar.gz"
    local tmp_dir=$(mktemp -d)
    local tmp_file="$tmp_dir/ntnt.tar.gz"

    echo "Downloading: $url"
    echo ""

    if download_file "$url" "$tmp_file"; then
        # Verify the download is a valid gzip file
        if ! gzip -t "$tmp_file" 2>/dev/null; then
            rm -rf "$tmp_dir"
            echo -e "${YELLOW}Downloaded file is corrupted. Will build from source.${NC}"
            return 1
        fi

        # Extract
        if ! tar -xzf "$tmp_file" -C "$tmp_dir" 2>/dev/null; then
            rm -rf "$tmp_dir"
            echo -e "${YELLOW}Failed to extract archive. Will build from source.${NC}"
            return 1
        fi

        # Verify binary exists and is executable
        if [ ! -f "$tmp_dir/ntnt" ]; then
            rm -rf "$tmp_dir"
            echo -e "${YELLOW}Binary not found in archive. Will build from source.${NC}"
            return 1
        fi

        # Install to ~/.local/bin
        mkdir -p "$INSTALL_DIR"
        mv "$tmp_dir/ntnt" "$INSTALL_DIR/ntnt"
        chmod +x "$INSTALL_DIR/ntnt"

        rm -rf "$tmp_dir"

        # Verify it runs
        if "$INSTALL_DIR/ntnt" --version &>/dev/null; then
            echo -e "${GREEN}[OK] Downloaded and installed ntnt to $INSTALL_DIR${NC}"
            return 0
        else
            echo -e "${YELLOW}Binary downloaded but won't run on this system. Will build from source.${NC}"
            rm -f "$INSTALL_DIR/ntnt"
            return 1
        fi
    else
        rm -rf "$tmp_dir"
        echo -e "${YELLOW}Download failed (release may not exist yet). Will build from source.${NC}"
        return 1
    fi
}

# Build from source (fallback)
build_from_source() {
    echo ""
    echo "Building from source..."
    echo ""

    # macOS: Check for Xcode Command Line Tools
    if [[ "$OSTYPE" == "darwin"* ]]; then
        if ! xcode-select -p &> /dev/null || ! xcrun --version &> /dev/null; then
            echo -e "${RED}X Xcode Command Line Tools not found${NC}"
            echo ""
            echo "Install them by running:"
            echo ""
            echo -e "  ${GREEN}xcode-select --install${NC}"
            echo ""
            exit 1
        fi
        echo -e "${GREEN}[OK] Xcode Command Line Tools found${NC}"
    fi

    # Linux: Check for C compiler
    if [[ "$OSTYPE" == "linux"* ]]; then
        if ! check_command cc && ! check_command gcc && ! check_command clang; then
            echo -e "${RED}X C compiler not found${NC}"
            echo ""
            if check_command apt-get; then
                echo -e "Install with: ${GREEN}sudo apt-get install build-essential${NC}"
            elif check_command dnf; then
                echo -e "Install with: ${GREEN}sudo dnf groupinstall 'Development Tools'${NC}"
            elif check_command yum; then
                echo -e "Install with: ${GREEN}sudo yum groupinstall 'Development Tools'${NC}"
            elif check_command pacman; then
                echo -e "Install with: ${GREEN}sudo pacman -S base-devel${NC}"
            elif check_command zypper; then
                echo -e "Install with: ${GREEN}sudo zypper install -t pattern devel_basis${NC}"
            else
                echo "Please install a C compiler (gcc or clang) using your package manager."
            fi
            echo ""
            exit 1
        fi
        echo -e "${GREEN}[OK] C compiler found${NC}"
    fi

    # Check for git
    if ! check_command git; then
        echo -e "${RED}X Git not found${NC}"
        echo ""
        if [[ "$OSTYPE" == "darwin"* ]]; then
            echo -e "Install with: ${GREEN}xcode-select --install${NC}"
        elif check_command apt-get; then
            echo -e "Install with: ${GREEN}sudo apt-get install git${NC}"
        elif check_command dnf; then
            echo -e "Install with: ${GREEN}sudo dnf install git${NC}"
        elif check_command yum; then
            echo -e "Install with: ${GREEN}sudo yum install git${NC}"
        elif check_command pacman; then
            echo -e "Install with: ${GREEN}sudo pacman -S git${NC}"
        else
            echo "Please install git using your package manager."
        fi
        echo ""
        exit 1
    fi
    echo -e "${GREEN}[OK] Git found${NC}"

    # Check for Rust/Cargo
    if ! check_command cargo; then
        echo -e "${YELLOW}Rust not found. Installing via rustup...${NC}"
        echo ""

        if ! check_command curl && ! check_command wget; then
            echo -e "${RED}Neither curl nor wget found. Cannot install Rust.${NC}"
            echo "Please install curl or wget, then re-run this installer."
            exit 1
        fi

        if check_command curl; then
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        else
            wget -qO- https://sh.rustup.rs | sh -s -- -y
        fi

        source "$HOME/.cargo/env"
        echo -e "${GREEN}[OK] Rust installed${NC}"
    else
        echo -e "${GREEN}[OK] Rust found: $(rustc --version)${NC}"
    fi

    # Clone or update repo
    NTNT_DIR="$(pwd)/ntnt-src"
    if [ -d "$NTNT_DIR" ]; then
        echo "Updating NTNT source in ./ntnt-src..."
        cd "$NTNT_DIR"
        git fetch --quiet origin
        git reset --quiet --hard origin/main
        git clean --quiet -fd
    else
        echo "Cloning NTNT source to ./ntnt-src..."
        git clone --quiet "https://github.com/$REPO.git" "$NTNT_DIR"
        cd "$NTNT_DIR"
    fi

    # Build and install
    echo "Building (this may take a few minutes)..."
    cargo install --path . --locked --quiet

    # cargo install puts it in ~/.cargo/bin
    INSTALL_DIR="$HOME/.cargo/bin"

    echo -e "${GREEN}[OK] Built and installed ntnt to $INSTALL_DIR${NC}"
}

# Main installation
echo "Checking for pre-built binary..."
echo ""

if try_download_binary; then
    INSTALLED_FROM="binary"
else
    build_from_source
    INSTALLED_FROM="source"
fi

echo ""
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  NTNT installed successfully!${NC}"
echo -e "${GREEN}============================================${NC}"
echo ""

# Show version
if [ -x "$INSTALL_DIR/ntnt" ]; then
    echo "Version: $($INSTALL_DIR/ntnt --version)"
elif check_command ntnt; then
    echo "Version: $(ntnt --version)"
fi
echo ""

# Check if in PATH
if ! check_command ntnt; then
    echo -e "${YELLOW}NOTE: Add ntnt to your PATH to use it from anywhere.${NC}"
    echo ""
    if [ "$INSTALL_DIR" = "$HOME/.local/bin" ]; then
        echo "  Add this to your shell profile (~/.zshrc, ~/.bashrc, etc.):"
        echo ""
        echo -e "    ${CYAN}export PATH=\"\$HOME/.local/bin:\$PATH\"${NC}"
    else
        echo "  Add this to your shell profile:"
        echo ""
        echo -e "    ${CYAN}export PATH=\"$INSTALL_DIR:\$PATH\"${NC}"
    fi
    echo ""
    echo "  Then restart your terminal or run: source ~/.zshrc (or ~/.bashrc)"
    echo ""
fi

echo "Get started:"
echo -e "  ${CYAN}ntnt run hello.tnt${NC}     # Run a file"
echo -e "  ${CYAN}ntnt --help${NC}            # See all commands"
echo ""
if [ "$INSTALLED_FROM" = "source" ]; then
    echo "Examples: ./ntnt-src/examples/"
fi
echo "Docs: https://github.com/$REPO"
echo ""

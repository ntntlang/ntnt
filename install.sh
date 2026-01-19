#!/bin/bash
# NTNT Language Installer
# Usage: curl -sSf https://raw.githubusercontent.com/ntntlang/ntnt/main/install.sh | bash

set -e

YELLOW='\033[1;33m'
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo ""
echo "🚀 Installing NTNT Language..."
echo ""

# macOS: Check for Xcode Command Line Tools
if [[ "$OSTYPE" == "darwin"* ]]; then
    # Check both that xcode-select has a path AND that the tools actually work
    if ! xcode-select -p &> /dev/null || ! xcrun --version &> /dev/null; then
        echo -e "${RED}❌ Xcode Command Line Tools not found or broken${NC}"
        echo ""
        echo "NTNT requires the Xcode Command Line Tools to compile on macOS."
        echo ""
        echo "Install or reinstall them by running:"
        echo ""
        echo -e "  ${GREEN}xcode-select --install${NC}"
        echo ""
        echo "If that doesn't work, try removing and reinstalling:"
        echo ""
        echo -e "  ${GREEN}sudo rm -rf /Library/Developer/CommandLineTools${NC}"
        echo -e "  ${GREEN}xcode-select --install${NC}"
        echo ""
        echo "After installation completes, re-run this installer."
        echo ""
        exit 1
    fi
    echo -e "${GREEN}✓ Xcode Command Line Tools found${NC}"
fi

# Linux: Check for C compiler (build-essential)
if [[ "$OSTYPE" == "linux"* ]]; then
    if ! command -v cc &> /dev/null && ! command -v gcc &> /dev/null; then
        echo -e "${RED}❌ C compiler not found${NC}"
        echo ""
        echo "NTNT requires a C compiler to build on Linux."
        echo ""
        if command -v apt-get &> /dev/null; then
            echo "Install build tools by running:"
            echo ""
            echo -e "  ${GREEN}sudo apt-get install build-essential${NC}"
        elif command -v dnf &> /dev/null; then
            echo "Install build tools by running:"
            echo ""
            echo -e "  ${GREEN}sudo dnf groupinstall 'Development Tools'${NC}"
        elif command -v pacman &> /dev/null; then
            echo "Install build tools by running:"
            echo ""
            echo -e "  ${GREEN}sudo pacman -S base-devel${NC}"
        else
            echo "Please install gcc/build tools using your system's package manager."
        fi
        echo ""
        echo "After installation completes, re-run this installer."
        echo ""
        exit 1
    fi
    echo -e "${GREEN}✓ C compiler found${NC}"
fi

# Check for git
if ! command -v git &> /dev/null; then
    echo -e "${RED}❌ Git not found${NC}"
    echo ""
    echo "NTNT requires git to download the source code."
    echo ""
    if [[ "$OSTYPE" == "darwin"* ]]; then
        echo "Install it by running:"
        echo ""
        echo -e "  ${GREEN}xcode-select --install${NC}"
    elif command -v apt-get &> /dev/null; then
        echo "Install it by running:"
        echo ""
        echo -e "  ${GREEN}sudo apt-get install git${NC}"
    elif command -v dnf &> /dev/null; then
        echo "Install it by running:"
        echo ""
        echo -e "  ${GREEN}sudo dnf install git${NC}"
    else
        echo "Please install git using your system's package manager."
    fi
    echo ""
    exit 1
fi

# Check for Rust/Cargo
if ! command -v cargo &> /dev/null; then
    echo -e "${YELLOW}Rust not found. Installing via rustup...${NC}"
    echo ""
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    echo ""
    echo -e "${GREEN}✓ Rust installed${NC}"
else
    echo -e "${GREEN}✓ Rust found: $(rustc --version)${NC}"
fi

# Clone or update repo in current directory
NTNT_DIR="$(pwd)/ntnt-src"
if [ -d "$NTNT_DIR" ]; then
    echo "Updating NTNT source in ./ntnt-src..."
    cd "$NTNT_DIR"
    # Always reset to match remote (handles force pushes, conflicts, etc.)
    git fetch --quiet origin
    git reset --quiet --hard origin/main
    git clean --quiet -fd
else
    echo "Downloading NTNT source to ./ntnt-src..."
    git clone --quiet https://github.com/ntntlang/ntnt.git "$NTNT_DIR"
    cd "$NTNT_DIR"
fi

# Build and install
echo "Building and installing to ~/.cargo/bin/ntnt..."
cargo install --path . --locked --quiet

echo ""
echo -e "${GREEN}✓ NTNT installed successfully!${NC}"
echo ""
echo "Version: $($HOME/.cargo/bin/ntnt --version)"
echo ""

# Check if ntnt is in PATH
if ! command -v ntnt &> /dev/null; then
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${YELLOW}  To use 'ntnt' command, add cargo's bin directory to your PATH.${NC}"
    echo -e "${YELLOW}  Run ONE of these commands based on your shell:${NC}"
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo "  Zsh (default on macOS):"
    echo -e "    ${GREEN}echo 'export PATH=\"\$HOME/.cargo/bin:\$PATH\"' >> ~/.zshrc && source ~/.zshrc${NC}"
    echo ""
    echo "  Bash:"
    echo -e "    ${GREEN}echo 'export PATH=\"\$HOME/.cargo/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc${NC}"
    echo ""
    echo "  Fish:"
    echo -e "    ${GREEN}fish_add_path ~/.cargo/bin${NC}"
    echo ""
    echo "  Or just restart your terminal."
    echo ""
fi

echo "Get started:"
echo -e "  ${GREEN}ntnt run hello.tnt${NC}     # Run a file"
echo -e "  ${GREEN}ntnt --help${NC}            # See all commands"
echo ""
echo "Examples are available in ./ntnt-src/examples/"
echo -e "  ${GREEN}ls ntnt-src/examples/${NC}"
echo ""
echo "Learn more: https://github.com/ntntlang/ntnt"
echo ""

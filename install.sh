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

# Clone or update repo
NTNT_DIR="$HOME/.ntnt-src"
if [ -d "$NTNT_DIR" ]; then
    echo "Updating existing installation..."
    cd "$NTNT_DIR"
    git pull --quiet
else
    echo "Downloading NTNT..."
    git clone --quiet https://github.com/ntntlang/ntnt.git "$NTNT_DIR"
    cd "$NTNT_DIR"
fi

# Build and install
echo "Building NTNT (this may take a minute)..."
cargo install --path . --locked --quiet

echo ""
echo -e "${GREEN}✓ NTNT installed successfully!${NC}"
echo ""

# Check if ntnt is accessible
if command -v ntnt &> /dev/null; then
    echo "Version: $(ntnt --version)"
else
    echo -e "${YELLOW}Note: You may need to add cargo's bin directory to your PATH.${NC}"
    echo ""
    echo "Add this to your ~/.zshrc or ~/.bashrc:"
    echo ""
    echo '  export PATH="$HOME/.cargo/bin:$PATH"'
    echo ""
    echo "Then restart your terminal or run: source ~/.zshrc"
fi

echo ""
echo "Get started:"
echo '  echo '\''print("Hello, World!")'\'' > hello.tnt'
echo "  ntnt run hello.tnt"
echo ""
echo "Learn more: https://github.com/ntntlang/ntnt"
echo ""

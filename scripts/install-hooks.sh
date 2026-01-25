#!/bin/bash
# Install git hooks for development

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

echo "Installing git hooks..."

cp "$SCRIPT_DIR/pre-commit" "$REPO_ROOT/.git/hooks/pre-commit"
chmod +x "$REPO_ROOT/.git/hooks/pre-commit"

echo "✅ Hooks installed!"
echo "   - pre-commit: format check + auto-generate docs"

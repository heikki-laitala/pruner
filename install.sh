#!/bin/bash
# Pruner installer — downloads pre-built binary and sets up project integration.
#
# Usage:
#   curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | bash
#   curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | bash -s -- --hook
#   curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | bash -s -- --copilot-skill
#   curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | bash -s -- --copilot-hook
#
# Options (pass after --):
#   --hook      Install Claude Code prompt-submit hook (better performance)
#   --global    Install skill/hook globally (~/.claude/) instead of project-local
#   --copilot-skill  Install Copilot CLI skill and instructions
#   --copilot-hook   Install Copilot userPromptSubmitted hook files
#   --copilot-global Install Copilot CLI skill globally (~/.copilot/)
#   --dir DIR   Install binary to DIR instead of ~/.local/bin
#   --version V Install specific version (default: latest)

set -euo pipefail

REPO="heikki-laitala/pruner"
INSTALL_DIR="${HOME}/.local/bin"
VERSION="latest"
HOOK=false
GLOBAL=false
COPILOT_SKILL=false
COPILOT_HOOK=false
COPILOT_GLOBAL=false

# Parse arguments
while [ $# -gt 0 ]; do
    case "$1" in
        --hook) HOOK=true ;;
        --global) GLOBAL=true ;;
        --copilot-skill) COPILOT_SKILL=true ;;
        --copilot-hook) COPILOT_HOOK=true ;;
        --copilot-global) COPILOT_GLOBAL=true ;;
        --dir) INSTALL_DIR="$2"; shift ;;
        --version) VERSION="$2"; shift ;;
        --help|-h)
            echo "Usage: install.sh [--hook] [--global] [--copilot-skill] [--copilot-hook] [--copilot-global] [--dir DIR] [--version VERSION]"
            echo ""
            echo "  --hook      Install Claude Code prompt-submit hook (better performance)"
            echo "  --global    Install skill/hook globally (~/.claude/)"
            echo "  --copilot-skill  Install Copilot CLI skill and instructions"
            echo "  --copilot-hook   Install Copilot userPromptSubmitted hook files"
            echo "  --copilot-global Install Copilot CLI skill globally (~/.copilot/)"
            echo "  --dir DIR   Install binary to DIR (default: ~/.local/bin)"
            echo "  --version V Install specific version (default: latest)"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
    shift
done

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)  OS_LABEL="linux" ;;
    Darwin) OS_LABEL="macos" ;;
    *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
    x86_64|amd64)  ARCH_LABEL="x86_64" ;;
    aarch64|arm64) ARCH_LABEL="aarch64" ;;
    *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

BINARY_NAME="pruner-${OS_LABEL}-${ARCH_LABEL}"

# Resolve version
if [ "$VERSION" = "latest" ]; then
    echo "Fetching latest release..."
    VERSION=$(curl -sSf "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')
    if [ -z "$VERSION" ]; then
        echo "Failed to fetch latest version. Check https://github.com/${REPO}/releases"
        exit 1
    fi
fi

echo "Installing pruner ${VERSION} (${OS_LABEL}/${ARCH_LABEL})..."

# Download binary
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${BINARY_NAME}.tar.gz"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

echo "Downloading ${DOWNLOAD_URL}..."
if ! curl -sSfL "$DOWNLOAD_URL" -o "${TEMP_DIR}/pruner.tar.gz"; then
    # Try without .tar.gz (plain binary)
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${BINARY_NAME}"
    if ! curl -sSfL "$DOWNLOAD_URL" -o "${TEMP_DIR}/pruner"; then
        echo "Download failed. Check https://github.com/${REPO}/releases"
        exit 1
    fi
else
    tar -xzf "${TEMP_DIR}/pruner.tar.gz" -C "$TEMP_DIR"
    mv "${TEMP_DIR}/${BINARY_NAME}" "${TEMP_DIR}/pruner" 2>/dev/null || true
fi

# Install binary
mkdir -p "$INSTALL_DIR"
cp "${TEMP_DIR}/pruner" "${INSTALL_DIR}/pruner"
chmod +x "${INSTALL_DIR}/pruner"
echo "Installed binary -> ${INSTALL_DIR}/pruner"

# Check PATH
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    echo ""
    echo "WARNING: ${INSTALL_DIR} is not in your PATH."
    echo "Add this to your shell profile (~/.zshrc or ~/.bashrc):"
    echo ""
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo ""
fi

# Verify
if command -v pruner >/dev/null 2>&1; then
    echo ""
    pruner --version
fi

# Set up project integration
echo ""
INIT_ARGS=""
if [ "$HOOK" = true ]; then
    INIT_ARGS="--hook"
fi
if [ "$GLOBAL" = true ]; then
    INIT_ARGS="${INIT_ARGS} --global"
fi
if [ "$COPILOT_SKILL" = true ]; then
    INIT_ARGS="${INIT_ARGS} --copilot-skill"
fi
if [ "$COPILOT_HOOK" = true ]; then
    INIT_ARGS="${INIT_ARGS} --copilot-hook"
fi
if [ "$COPILOT_GLOBAL" = true ]; then
    INIT_ARGS="${INIT_ARGS} --copilot-global"
fi

if [ "$GLOBAL" = true ] || [ "$COPILOT_GLOBAL" = true ]; then
    echo "Setting up global integration..."
    "${INSTALL_DIR}/pruner" init ${INIT_ARGS}
else
    echo "To set up pruner in a project:"
    echo ""
    if [ "$HOOK" = true ]; then
        echo "  pruner init /path/to/project --hook   # Claude Code (best performance)"
    else
        echo "  pruner init /path/to/project          # skill mode (works everywhere)"
        echo "  pruner init /path/to/project --hook   # Claude Code (best performance)"
    fi
    if [ "$COPILOT_SKILL" = true ]; then
        echo "  pruner init /path/to/project --copilot-skill   # Copilot CLI skill files"
    else
        echo "  pruner init /path/to/project --copilot-skill   # Copilot CLI integration"
    fi
    if [ "$COPILOT_HOOK" = true ]; then
        echo "  pruner init /path/to/project --copilot-hook    # Copilot prompt hook files"
    else
        echo "  pruner init /path/to/project --copilot-hook    # Copilot prompt hook integration"
    fi
    echo "  pruner index /path/to/project          # index the codebase"
fi

echo ""
echo "Done."

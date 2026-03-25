#!/bin/bash
# Pruner installer — downloads pre-built binary and sets up project integration.
#
# Interactive:
#   curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | bash
#
# Non-interactive (flags skip the prompts):
#   curl -sSf ... | bash -s -- --hook --global
#   curl -sSf ... | bash -s -- --copilot-skill --copilot-global
#
# Options (pass after --):
#   --hook           Install Claude Code prompt-submit hook (better performance)
#   --global         Install skill/hook globally (~/.claude/) instead of project-local
#   --copilot-skill  Install Copilot CLI skill and instructions
#   --copilot-hook   Install Copilot userPromptSubmitted hook files
#   --copilot-global Install Copilot CLI skill globally (~/.copilot/)
#   --dir DIR        Install binary to DIR instead of ~/.local/bin
#   --version V      Install specific version (default: latest)
#   --no-interactive Skip interactive prompts (just install binary)

set -euo pipefail

REPO="heikki-laitala/pruner"
INSTALL_DIR="${HOME}/.local/bin"
VERSION="latest"
HOOK=false
GLOBAL=false
COPILOT_SKILL=false
COPILOT_HOOK=false
COPILOT_GLOBAL=false
NO_INTERACTIVE=false
HAS_SETUP_FLAGS=false

# Parse arguments
while [ $# -gt 0 ]; do
    case "$1" in
        --hook) HOOK=true; HAS_SETUP_FLAGS=true ;;
        --global) GLOBAL=true; HAS_SETUP_FLAGS=true ;;
        --copilot-skill) COPILOT_SKILL=true; HAS_SETUP_FLAGS=true ;;
        --copilot-hook) COPILOT_HOOK=true; HAS_SETUP_FLAGS=true ;;
        --copilot-global) COPILOT_GLOBAL=true; HAS_SETUP_FLAGS=true ;;
        --no-interactive) NO_INTERACTIVE=true ;;
        --dir) INSTALL_DIR="$2"; shift ;;
        --version) VERSION="$2"; shift ;;
        --help|-h)
            echo "Usage: install.sh [--hook] [--global] [--copilot-skill] [--copilot-hook] [--copilot-global] [--dir DIR] [--version VERSION] [--no-interactive]"
            echo ""
            echo "  --hook           Install Claude Code prompt-submit hook (better performance)"
            echo "  --global         Install skill/hook globally (~/.claude/)"
            echo "  --copilot-skill  Install Copilot CLI skill and instructions"
            echo "  --copilot-hook   Install Copilot userPromptSubmitted hook files"
            echo "  --copilot-global Install Copilot CLI skill globally (~/.copilot/)"
            echo "  --dir DIR        Install binary to DIR (default: ~/.local/bin)"
            echo "  --version V      Install specific version (default: latest)"
            echo "  --no-interactive Skip interactive prompts (just install binary)"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
    shift
done

# Helper: read user input from /dev/tty (works even when stdin is piped from curl)
ask() {
    local prompt="$1"
    local default="$2"
    local reply
    if [ -r /dev/tty ]; then
        printf "%s " "$prompt" > /dev/tty
        read -r reply < /dev/tty
    else
        reply=""
    fi
    echo "${reply:-$default}"
}

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

# ── Interactive setup (when no flags provided) ──────────────────────────

if [ "$HAS_SETUP_FLAGS" = false ] && [ "$NO_INTERACTIVE" = false ] && [ -r /dev/tty ]; then
    echo ""
    echo "How do you want to set up pruner?"
    echo ""
    echo "  1) Claude Code  (global — works in every repo)"
    echo "  2) Copilot CLI  (global — works in every repo)"
    echo "  3) Both Claude Code + Copilot CLI (global)"
    echo "  4) Skip — I'll set up later with 'pruner init'"
    echo ""
    CHOICE=$(ask "Choice [1]:" "1")

    case "$CHOICE" in
        1)
            GLOBAL=true
            echo ""
            echo "Claude Code mode:"
            echo "  1) Hook — context injected automatically (best performance)"
            echo "  2) Skill — Claude calls pruner as a tool"
            echo ""
            MODE=$(ask "Choice [1]:" "1")
            if [ "$MODE" = "1" ]; then
                HOOK=true
            fi
            ;;
        2)
            COPILOT_GLOBAL=true
            COPILOT_SKILL=true
            ;;
        3)
            GLOBAL=true
            COPILOT_GLOBAL=true
            COPILOT_SKILL=true
            echo ""
            echo "Claude Code mode:"
            echo "  1) Hook — context injected automatically (best performance)"
            echo "  2) Skill — Claude calls pruner as a tool"
            echo ""
            MODE=$(ask "Choice [1]:" "1")
            if [ "$MODE" = "1" ]; then
                HOOK=true
            fi
            ;;
        4)
            echo ""
            echo "To set up later:"
            echo "  pruner init --global --hook          # Claude Code (recommended)"
            echo "  pruner init --copilot-skill --copilot-global  # Copilot CLI"
            echo "  pruner init /path/to/project --hook  # per-project"
            echo ""
            echo "Done."
            exit 0
            ;;
        *)
            echo "Invalid choice. Skipping setup."
            echo ""
            echo "Done."
            exit 0
            ;;
    esac
fi

# ── Run pruner init ─────────────────────────────────────────────────────

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
    echo ""
    echo "Pruner is ready. It will auto-index each repo on first use."
    echo "Add .pruner/ to your .gitignore (global install doesn't modify it):"
    echo ""
    echo "  echo '.pruner/' >> .gitignore"
elif [ "$HAS_SETUP_FLAGS" = true ]; then
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
    fi
    if [ "$COPILOT_HOOK" = true ]; then
        echo "  pruner init /path/to/project --copilot-hook    # Copilot prompt hook files"
    fi
fi

echo ""
echo "Done."

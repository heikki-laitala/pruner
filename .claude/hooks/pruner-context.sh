#!/bin/bash
# UserPromptSubmit hook: runs pruner context and injects output into conversation.
# Stdin: JSON with .prompt field. Stdout: injected as additional context.
# Works on macOS, Linux, and Windows (via Git Bash).

INPUT=$(cat)

# Extract .prompt from JSON — try jq first, fall back to sed
if command -v jq >/dev/null 2>&1; then
  PROMPT=$(echo "$INPUT" | jq -r '.prompt // empty')
else
  PROMPT=$(echo "$INPUT" | sed -n 's/.*"prompt" *: *"\(.*\)"/\1/p' | sed 's/",".*//; s/",.*//')
fi

if [ -z "$PROMPT" ]; then
  exit 0
fi

# Find pruner binary: PATH first, then common install locations, then dev build
PRUNER=$(command -v pruner 2>/dev/null)
if [ -z "$PRUNER" ]; then
  SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd)"
  candidates=(
    "$HOME/.local/bin/pruner"
    "$HOME/.local/bin/pruner.exe"
    "$HOME/.cargo/bin/pruner"
    "$SCRIPT_DIR/../../target/release/pruner"
    "$SCRIPT_DIR/../../target/release/pruner.exe"
  )
  # Unix-only install locations
  if [ -d "/usr/local/bin" ]; then
    candidates+=("/usr/local/bin/pruner")
  fi
  # Windows: check default install dir
  if [ -n "$USERPROFILE" ]; then
    candidates+=("$USERPROFILE/.local/bin/pruner.exe")
  fi
  for candidate in "${candidates[@]}"; do
    if [ -f "$candidate" ]; then
      PRUNER="$candidate"
      break
    fi
  done
fi

if [ -z "$PRUNER" ] || [ ! -f "$PRUNER" ]; then
  exit 0
fi

# Run pruner context on the project directory
REPO="${CLAUDE_PROJECT_DIR:-.}"

# Only run if this looks like a code repo or meta-repo with indexed sub-repos.
# Avoids creating .pruner/ in random directories like ~ or ~/Downloads.
HAS_INDEX=false
if [ -e "$REPO/.git" ] || [ -d "$REPO/.pruner" ]; then
  HAS_INDEX=true
fi
# Check for sub-repos (meta-repo pattern): child dirs with .git or .pruner/index.db
if [ "$HAS_INDEX" = false ]; then
  for d in "$REPO"/*/; do
    if [ -e "${d}.git" ] || [ -f "${d}.pruner/index.db" ]; then
      HAS_INDEX=true
      break
    fi
  done
fi
if [ "$HAS_INDEX" = false ]; then
  exit 0
fi

OUTPUT=$("$PRUNER" context "$REPO" "$PROMPT" 2>/dev/null)

if [ -n "$OUTPUT" ]; then
  echo "## Pruner context (pre-computed codebase analysis)"
  echo ""
  echo "$OUTPUT"
fi

exit 0

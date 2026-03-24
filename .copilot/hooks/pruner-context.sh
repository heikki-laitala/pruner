#!/bin/bash
# Copilot userPromptSubmitted hook: runs pruner context and stores output in .pruner/copilot-context.md
# Input JSON via stdin with .prompt and .cwd.

set -euo pipefail

INPUT=$(cat)

# Extract .prompt and .cwd from JSON without jq (pure bash/sed)
PROMPT=$(echo "$INPUT" | sed -n 's/.*"prompt" *: *"\(.*\)"/\1/p' | sed 's/\\"/"/g')
# Handle multi-field lines: strip trailing comma and any fields after prompt
PROMPT=$(echo "$PROMPT" | sed 's/",".*//; s/",.*//')

if [ -z "$PROMPT" ]; then
  # Fallback: try jq if available
  if command -v jq >/dev/null 2>&1; then
    PROMPT=$(echo "$INPUT" | jq -r '.prompt // empty')
  fi
fi

if [ -z "$PROMPT" ]; then
  exit 0
fi

CWD=$(echo "$INPUT" | sed -n 's/.*"cwd" *: *"\([^"]*\)".*/\1/p')
if [ -z "$CWD" ]; then
  if command -v jq >/dev/null 2>&1; then
    CWD=$(echo "$INPUT" | jq -r '.cwd // "."')
  else
    CWD="."
  fi
fi

ROOT="${CWD}"
if [ ! -d "${ROOT}" ]; then
  ROOT="."
fi

PRUNER=$(command -v pruner 2>/dev/null || true)
if [ -z "$PRUNER" ]; then
  for candidate in \
    "$HOME/.local/bin/pruner" \
    "$HOME/.local/bin/pruner.exe" \
    "$HOME/.cargo/bin/pruner" \
    "${ROOT}/target/release/pruner" \
    "${ROOT}/target/release/pruner.exe"; do
    if [ -f "$candidate" ]; then
      PRUNER="$candidate"
      break
    fi
  done
  # Unix-only install location
  if [ -z "$PRUNER" ] && [ -f "/usr/local/bin/pruner" ]; then
    PRUNER="/usr/local/bin/pruner"
  fi
  # Windows: check default install dir
  if [ -z "$PRUNER" ] && [ -n "$USERPROFILE" ] && [ -f "$USERPROFILE/.local/bin/pruner.exe" ]; then
    PRUNER="$USERPROFILE/.local/bin/pruner.exe"
  fi
fi
if [ -z "$PRUNER" ] || [ ! -f "$PRUNER" ]; then
  exit 0
fi

mkdir -p "${ROOT}/.pruner"
OUT_FILE="${ROOT}/.pruner/copilot-context.md"

OUTPUT=$("$PRUNER" context "$ROOT" "$PROMPT" 2>/dev/null || true)
if [ -z "$OUTPUT" ]; then
  exit 0
fi

{
  echo "# Pruner context (pre-computed codebase analysis)"
  echo
  echo "$OUTPUT"
  echo
  echo "Use this context to work directly. Only read source files if a snippet is truncated."
  echo "Do not re-explore with grep/glob for the same keywords."
} > "$OUT_FILE"

exit 0

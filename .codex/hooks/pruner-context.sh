#!/bin/bash
# Codex UserPromptSubmit hook: runs pruner context and injects output as extra developer context.
# Stdin: JSON with .prompt and .cwd. Stdout: plain text added as developer context.

INPUT=$(cat)

if command -v jq >/dev/null 2>&1; then
  PROMPT=$(echo "$INPUT" | jq -r '.prompt // empty')
  CWD=$(echo "$INPUT" | jq -r '.cwd // "."')
elif command -v python3 >/dev/null 2>&1; then
  PROMPT=$(echo "$INPUT" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get("prompt",""))' 2>/dev/null)
  CWD=$(echo "$INPUT" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get("cwd","."))' 2>/dev/null)
else
  # No jq or python3 — cannot safely parse JSON
  exit 0
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
  if [ -d "/usr/local/bin" ]; then
    candidates+=("/usr/local/bin/pruner")
  fi
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

REPO="${CWD:-.}"

HAS_INDEX=false
if [ -e "$REPO/.git" ] || [ -d "$REPO/.pruner" ]; then
  HAS_INDEX=true
fi
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

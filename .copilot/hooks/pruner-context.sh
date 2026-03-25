#!/bin/bash
# Copilot userPromptSubmitted hook: runs pruner context and stores output in .pruner/copilot-context.md
# Input JSON via stdin with .prompt and .cwd.
# Works on macOS, Linux, and Windows (via Git Bash).

INPUT=$(cat)

# Extract .prompt from JSON — try jq first, fall back to sed
if command -v jq >/dev/null 2>&1; then
  PROMPT=$(echo "$INPUT" | jq -r '.prompt // empty')
  CWD=$(echo "$INPUT" | jq -r '.cwd // "."')
else
  PROMPT=$(echo "$INPUT" | sed -n 's/.*"prompt" *: *"\(.*\)"/\1/p' | sed 's/",".*//; s/",.*//')
  CWD=$(echo "$INPUT" | sed -n 's/.*"cwd" *: *"\([^"]*\)".*/\1/p')
  [ -z "$CWD" ] && CWD="."
fi

if [ -z "$PROMPT" ]; then
  exit 0
fi

ROOT="${CWD}"
if [ ! -d "${ROOT}" ]; then
  ROOT="."
fi

# Only run if this looks like a code repo or meta-repo with indexed sub-repos.
# Avoids creating .pruner/ in random directories like ~ or ~/Downloads.
HAS_INDEX=false
if [ -e "$ROOT/.git" ] || [ -d "$ROOT/.pruner" ]; then
  HAS_INDEX=true
fi
# Check for sub-repos (meta-repo pattern): child dirs with .git or .pruner/index.db
if [ "$HAS_INDEX" = false ]; then
  for d in "$ROOT"/*/; do
    if [ -e "${d}.git" ] || [ -f "${d}.pruner/index.db" ]; then
      HAS_INDEX=true
      break
    fi
  done
fi
if [ "$HAS_INDEX" = false ]; then
  exit 0
fi

# Find pruner binary: PATH first, then common install locations, then dev build
PRUNER=$(command -v pruner 2>/dev/null || true)
if [ -z "$PRUNER" ]; then
  candidates=(
    "$HOME/.local/bin/pruner"
    "$HOME/.local/bin/pruner.exe"
    "$HOME/.cargo/bin/pruner"
    "${ROOT}/target/release/pruner"
    "${ROOT}/target/release/pruner.exe"
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

mkdir -p "${ROOT}/.pruner"
OUTPUT=$("$PRUNER" context "$ROOT" "$PROMPT" 2>/dev/null || true)

if [ -n "$OUTPUT" ]; then
  {
    echo "# Pruner context (pre-computed codebase analysis)"
    echo
    echo "$OUTPUT"
    echo
    echo "Use this context to work directly. Only read source files if a snippet is truncated."
    echo "Do not re-explore with grep/glob for the same keywords."
  } > "${ROOT}/.pruner/copilot-context.md"
fi

exit 0

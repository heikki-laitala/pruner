#!/bin/bash
# UserPromptSubmit hook: runs pruner context and injects output into conversation.
# Stdin: JSON with .prompt field. Stdout: injected as additional context.

INPUT=$(cat)
PROMPT=$(echo "$INPUT" | jq -r '.prompt // empty')

if [ -z "$PROMPT" ]; then
  exit 0
fi

# Find pruner binary
PRUNER=$(command -v pruner 2>/dev/null)
if [ -z "$PRUNER" ]; then
  PRUNER="$(dirname "$0")/../../target/release/pruner"
fi

if [ ! -x "$PRUNER" ]; then
  exit 0
fi

# Run pruner context on the project directory
REPO="${CLAUDE_PROJECT_DIR:-.}"
OUTPUT=$("$PRUNER" context "$REPO" "$PROMPT" 2>/dev/null)

if [ -n "$OUTPUT" ]; then
  echo "## Pruner context (pre-computed codebase analysis)"
  echo ""
  echo "$OUTPUT"
  echo ""
  echo "Use this context to work directly. Only read source files if a snippet is truncated. Do not re-explore with grep/glob for the same keywords."
fi

exit 0

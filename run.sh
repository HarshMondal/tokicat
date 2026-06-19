#!/usr/bin/env bash
# Launch the Claude Code pet. Quits any already-running instance first.
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pkill -f "python3 .*pet.py" 2>/dev/null || true
exec python3 "$REPO_DIR/pet.py"

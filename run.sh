#!/usr/bin/env bash
# Launch ZeroClaw using Claude Code's OAuth token from macOS Keychain.
# Usage: ./run.sh agent          (interactive agent)
#        ./run.sh agent -m "Hi"  (single message)
#        ./run.sh daemon         (full runtime)
#        ./run.sh status         (system status)

set -euo pipefail

# Extract Claude Code OAuth token from macOS Keychain
CREDS_JSON=$(security find-generic-password -s "Claude Code-credentials" -a "$(whoami)" -w 2>/dev/null) || {
  echo "Error: Could not read Claude Code credentials from Keychain."
  echo "Make sure you are logged into Claude Code."
  exit 1
}

TOKEN=$(echo "$CREDS_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['claudeAiOauth']['accessToken'])" 2>/dev/null) || {
  echo "Error: Could not extract OAuth token from Claude Code credentials."
  exit 1
}

if [[ -z "$TOKEN" ]]; then
  echo "Error: OAuth token is empty."
  exit 1
fi

export ANTHROPIC_OAUTH_TOKEN="$TOKEN"

exec cargo run --release -- "$@"

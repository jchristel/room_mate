#!/usr/bin/env bash
# Proves a built package works: unpacks it into a throwaway home, installs it, starts the server and
# asks it for the viewer and the project list, then checks registration prints the right command.
# Run by CI on each platform and usable by hand. Touches nothing outside a temp directory.
#
# Usage: installer/unix/smoke.sh <package.tar.gz>
set -euo pipefail

PKG=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
T=$(mktemp -d)
PID=
cleanup() { if [ -n "$PID" ]; then kill "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true; fi; rm -rf "$T"; }
trap cleanup EXIT

cd "$T"
tar xzf "$PKG"
export ROOMMATE_HOME=$T/home ROOMMATE_BIN=$T/bin HOME=$T/h
mkdir -p "$HOME"
./roommate-*/install.sh --no-claude >/dev/null

PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
"$ROOMMATE_BIN/roommate" --no-browser --port "$PORT" >"$T/server.log" 2>&1 &
PID=$!
for _ in $(seq 1 60); do curl -fs -o /dev/null "http://127.0.0.1:$PORT/" && break; sleep 0.5; done

code() { curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT$1"; }
[ "$(code /)" = 200 ] || { echo "viewer did not answer"; cat "$T/server.log"; exit 1; }
[ "$(code /projects)" = 200 ] || { echo "/projects did not answer"; exit 1; }
curl -fs "http://127.0.0.1:$PORT/vendor/renderer.bundle.js" -o /dev/null || { echo "viewer files were not found beside the binary"; exit 1; }

# An upgrade keeps the user's settings and data.
echo "# edited" >> "$ROOMMATE_HOME/settings/server.toml"
./roommate-*/install.sh --no-claude >/dev/null
grep -q '^# edited' "$ROOMMATE_HOME/settings/server.toml" || { echo "an install overwrote the settings"; exit 1; }

# `claude` need not exist: a dry run must still be right, and a missing one must not fail.
PATH=$T/fakebin:$PATH
mkdir -p "$T/fakebin"; printf '#!/bin/sh\nexit 0\n' > "$T/fakebin/claude"; chmod +x "$T/fakebin/claude"
"$ROOMMATE_HOME/app/register-claude-mcp.sh" --dry-run | grep -q 'mcp add roommate --scope user -- .*/app/mcp --server-settings' \
  || { echo "registration printed the wrong command"; exit 1; }
echo "smoke ok: $(basename "$PKG")"

#!/usr/bin/env bash
# Installs RoomMate (the server and both MCP servers; no Revit tooling) for the current user. No
# admin rights, nothing outside $HOME. The Unix twin of the Windows installer, and split the same
# way so upgrades are safe:
#
#   $ROOMMATE_HOME/app        replaced wholesale on every install (the binaries, viewer, launcher)
#   $ROOMMATE_HOME/settings   seeded once, never overwritten: the settings UI writes project files here
#   $ROOMMATE_HOME/data       the snapshot store; never touched
#
# ROOMMATE_HOME defaults to ~/.local/share/roommate. A `roommate` command is put in ~/.local/bin
# (override with ROOMMATE_BIN).
#
# Usage: ./install.sh [--no-claude]     (--no-claude: do not register with Claude Code)
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOMMATE_HOME:-$HOME/.local/share/roommate}
BIN=${ROOMMATE_BIN:-$HOME/.local/bin}
CLAUDE=1
for a in "$@"; do
  case "$a" in
    --no-claude) CLAUDE=0 ;;
    -h|--help) sed -n '2,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

for f in roommate mcp drofus-mcp roommate.sh register-claude-mcp.sh static settings-template; do
  [ -e "$HERE/$f" ] || { echo "$f is missing from $HERE -- is this the unpacked package?" >&2; exit 1; }
done

# Replaced, not merged: a file the new version dropped must not linger beside it.
rm -rf "$ROOT/app"
mkdir -p "$ROOT/app" "$ROOT/settings/projects" "$ROOT/data/snapshots" "$BIN"
cp -R "$HERE/." "$ROOT/app/"
rm -f "$ROOT/app/install.sh"

# onlyifdoesntexist, as on Windows: an edited project file survives an upgrade.
[ -f "$ROOT/settings/server.toml" ] || cp "$HERE/settings-template/server.toml" "$ROOT/settings/server.toml"
for p in "$HERE"/settings-template/projects/*.toml; do
  [ -f "$ROOT/settings/projects/$(basename "$p")" ] || cp "$p" "$ROOT/settings/projects/"
done

# A browser download is quarantined by macOS and the unsigned binaries then refuse to start.
# Clearing it is this user's own install of a file they chose to unpack.
if [ "$(uname -s)" = Darwin ]; then
  xattr -dr com.apple.quarantine "$ROOT/app" 2>/dev/null || true
fi

# A wrapper, not a symlink: the launcher finds its binaries from its own location, and resolving
# a symlink portably (readlink -f) is not something macOS has always had.
cat > "$BIN/roommate" <<WRAP
#!/bin/sh
export ROOMMATE_HOME="$ROOT"
exec "$ROOT/app/roommate.sh" "\$@"
WRAP
chmod +x "$BIN/roommate"

echo "Installed to $ROOT"
echo "Start the server and viewer with:  roommate"
case ":$PATH:" in *":$BIN:"*) ;; *) echo "Note: $BIN is not on your PATH; add it, or run $BIN/roommate." ;; esac

if [ "$CLAUDE" = 1 ]; then
  ROOMMATE_HOME="$ROOT" "$ROOT/app/register-claude-mcp.sh"
  echo "For dRofus (asks for your database, project and API key):  $ROOT/app/register-claude-mcp.sh --drofus"
fi

#!/usr/bin/env bash
# Starts the RoomMate server and opens the viewer. The Unix twin of installer/RoomMate.ps1, and
# deliberate in the same three ways: the server runs in the FOREGROUND (Ctrl-C stops it; a
# background start leaves an invisible process holding the port), the browser opens only once the
# port answers (opening at once races the start and lands on a connection error), and settings are
# re-seeded from the template when missing, so deleting the data folder is a reset.
#
# Usage: roommate [--port N] [--bind ADDR] [--no-browser]
set -euo pipefail

APP=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOMMATE_HOME:-$HOME/.local/share/roommate}
SETTINGS=$ROOT/settings
PORT=5151
BIND=127.0.0.1
BROWSER=1

while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT=$2; shift 2 ;;
    --bind) BIND=$2; shift 2 ;;
    --no-browser) BROWSER=0; shift ;;
    -h|--help) sed -n '2,10p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

URL=http://127.0.0.1:$PORT

# Seed-if-absent, never overwrite: the only thing between an edited project file and an upgrade.
if [ ! -f "$SETTINGS/server.toml" ]; then
  echo "Setting up $ROOT ..."
  mkdir -p "$SETTINGS"
  cp -R "$APP/settings-template/." "$SETTINGS/"
fi
mkdir -p "$ROOT/data/snapshots"

answers() { command -v curl >/dev/null 2>&1 && curl -fs -o /dev/null "$URL/"; }

if answers; then
  echo "RoomMate is already running on $URL."
  [ "$BROWSER" = 1 ] && { command -v open >/dev/null && open "$URL" || xdg-open "$URL" >/dev/null 2>&1 || true; }
  exit 0
fi

if [ "$BROWSER" = 1 ]; then
  (
    for _ in $(seq 1 240); do
      if answers; then
        command -v open >/dev/null && open "$URL" || xdg-open "$URL" >/dev/null 2>&1 || true
        exit 0
      fi
      sleep 0.25
    done
  ) &
fi

echo "RoomMate  ->  $URL"
echo "Ctrl-C stops the server."
echo
exec "$APP/roommate" --server-settings "$SETTINGS/server.toml" \
  --project-settings "$SETTINGS/projects" --port "$PORT" --bind "$BIND"

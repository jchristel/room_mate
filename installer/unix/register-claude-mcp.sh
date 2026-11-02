#!/usr/bin/env bash
# Registers RoomMate's MCP servers with Claude Code (`claude mcp add --scope user`), so they work
# with no setup by hand. The Unix twin of installer/Register-ClaudeMcp.ps1.
#
#   (no option)      the RoomMate data server (mcp), against this install's settings
#   --url URL        a RoomMate server somebody else runs on the network instead
#                    (docs/mcp-host-setup.md, 2b); add --token SECRET if it has one
#   --drofus         ask for region, database, project and API key; register drofus-mcp
#   --remove         take both back out
#   --dry-run        print what would run (the API key and token shown as ***)
#
# Registering replaces an entry of the same name, so running it again repairs it. A missing Claude
# Code is a message, not an error: the installer must not fail because a tool is absent.
set -euo pipefail

APP=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SETTINGS=${ROOMMATE_HOME:-$HOME/.local/share/roommate}/settings
URL= TOKEN= DROFUS=0 REMOVE=0 DRY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --url) URL=$2; shift 2 ;;
    --token) TOKEN=$2; shift 2 ;;
    --drofus) DROFUS=1; shift ;;
    --remove) REMOVE=1; shift ;;
    --dry-run) DRY=1; shift ;;
    -h|--help) sed -n '2,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

CLAUDE=$(command -v claude || true)
[ -z "$CLAUDE" ] && [ -x "$HOME/.local/bin/claude" ] && CLAUDE=$HOME/.local/bin/claude
if [ -z "$CLAUDE" ]; then
  [ "$REMOVE" = 1 ] && exit 0
  echo "Claude Code was not found, so nothing was registered."
  echo "Install it, then run: $APP/register-claude-mcp.sh"
  exit 0
fi

# run <shown...> @@ <real...>: prints the shown form on a dry run, else runs the real one. The
# separator is not "--" because the real arguments contain one themselves.
run() {
  local shown=() real=()
  while [ "$1" != "@@" ]; do shown+=("$1"); shift; done
  shift; real=("$@")
  if [ "$DRY" = 1 ]; then echo "claude ${shown[*]}"; return 0; fi
  "$CLAUDE" "${real[@]}" >/dev/null 2>&1
}

add() { # add <name> <shown args, then @@, then real args>
  local name=$1; shift
  run mcp remove "$name" --scope user @@ mcp remove "$name" --scope user || true
  if run "$@"; then [ "$DRY" = 1 ] || echo "Registered '$name' with Claude Code."
  else echo "Could not register '$name'. Run 'claude mcp add --help' to see what Claude Code expects." >&2; exit 1
  fi
}

if [ "$REMOVE" = 1 ]; then
  for name in roommate drofus; do run mcp remove "$name" --scope user @@ mcp remove "$name" --scope user || true; done
  exit 0
fi

if [ "$DROFUS" = 1 ]; then
  [ -x "$APP/drofus-mcp" ] || { echo "drofus-mcp was not found beside this script ($APP)." >&2; exit 1; }
  echo "dRofus: these three values come from your dRofus account."
  echo "Region host: api-no (default), api-eu, api-us, api-ca or api-au."
  read -r -p "Region [api-no]: " region; region=${region:-api-no}
  read -r -p "Database: " database
  read -r -p "Project: " project
  read -r -s -p "API key (not shown): " key; echo
  if [ -z "$database" ] || [ -z "$project" ] || [ -z "$key" ]; then
    echo "Database, project and API key are all needed; nothing was registered." >&2; exit 1
  fi
  tail=(-- "$APP/drofus-mcp" --base-url "https://$region.drofus.com" --database "$database" --project "$project")
  # The name goes first: --env takes any number of values and would swallow it otherwise.
  add drofus mcp add drofus --scope user --env "DROFUS_API_KEY=***" "${tail[@]}" \
    @@ mcp add drofus --scope user --env "DROFUS_API_KEY=$key" "${tail[@]}"
  echo "The API key is stored in Claude Code's configuration file."
  exit 0
fi

if [ -n "$URL" ]; then
  if [ -n "$TOKEN" ]; then
    add roommate mcp add --transport http --scope user roommate "$URL" --header "Authorization: Bearer ***" \
      @@ mcp add --transport http --scope user roommate "$URL" --header "Authorization: Bearer $TOKEN"
  else
    add roommate mcp add --transport http --scope user roommate "$URL" \
      @@ mcp add --transport http --scope user roommate "$URL"
  fi
  exit 0
fi

[ -x "$APP/mcp" ] || { echo "mcp was not found beside this script ($APP)." >&2; exit 1; }
args=(mcp add roommate --scope user -- "$APP/mcp" --server-settings "$SETTINGS/server.toml" --project-settings "$SETTINGS/projects")
add roommate "${args[@]}" @@ "${args[@]}"

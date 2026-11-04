#!/usr/bin/env bash
# Registers RoomMate's MCP servers with Claude Code (`claude mcp add --scope user`), so they work
# with no setup by hand. The Unix twin of installer/Register-ClaudeMcp.ps1.
#
#   (no option)      the RoomMate data server (mcp), against this install's settings
#   --url URL        a RoomMate server somebody else runs on the network instead
#                    (docs/mcp-host-setup.md, 2b); if it has a shared secret, add --ask-token
#                    (prompts, nothing shown) or set ROOMMATE_MCP_TOKEN. --token SECRET still
#                    works but is deprecated: a command line shows in history and `ps`
#   --drofus         ask for region, database, project and API key; register drofus-mcp. The key
#                    goes into the OS credential store (macOS) or else a file only you can read;
#                    Claude Code is given at most a path
#   --remove         take both back out
#   --dry-run        print what would run (the API key and token shown as ***)
#
# Registering replaces an entry of the same name, so running it again repairs it. A missing Claude
# Code is a message, not an error: the installer must not fail because a tool is absent.
set -euo pipefail

APP=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SETTINGS=${ROOMMATE_HOME:-$HOME/.local/share/roommate}/settings
# Where the dRofus key is kept: a file only this user can read, so Claude Code's configuration holds a path.
KEY_FILE=${XDG_CONFIG_HOME:-$HOME/.config}/roommate/secrets/drofus.key
URL= TOKEN=${ROOMMATE_MCP_TOKEN:-} ASK_TOKEN=0 DROFUS=0 REMOVE=0 DRY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --url) URL=$2; shift 2 ;;
    --token) TOKEN=$2; echo "warning: --token puts the secret in your shell history; use --ask-token" >&2; shift 2 ;;
    --ask-token) ASK_TOKEN=1; shift ;;
    --drofus) DROFUS=1; shift ;;
    --remove) REMOVE=1; shift ;;
    --dry-run) DRY=1; shift ;;
    -h|--help) sed -n '2,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
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
  if [ "$DRY" != 1 ]; then
    rm -f "$KEY_FILE"
    [ -x "$APP/drofus-mcp" ] && "$APP/drofus-mcp" --clear-key >/dev/null 2>&1 || true
  fi
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
  # The key is kept where Claude Code's own (plain text) configuration need not hold it. First choice: the OS
  # credential store, written by drofus-mcp itself with the key on standard input (macOS Keychain; Linux has none
  # it can rely on and says so). Fallback: a file only this user can read. Claude Code is told a path at most.
  env_args=(); where=
  if [ "$DRY" = 1 ]; then
    echo "(would store the API key in the OS credential store if there is one, else in $KEY_FILE, mode 600)"
    where="the OS credential store or $KEY_FILE"
  elif printf '%s\n' "$key" | "$APP/drofus-mcp" --store-key >/dev/null 2>&1; then
    where="the OS credential store"
  else
    ( umask 077; mkdir -p "$(dirname "$KEY_FILE")"; printf '%s\n' "$key" > "$KEY_FILE" )
    chmod 600 "$KEY_FILE"
    env_args=(--env "DROFUS_API_KEY_FILE=$KEY_FILE")
    where="$KEY_FILE (readable by you only)"
  fi
  # The name goes first: --env takes any number of values and would swallow it otherwise.
  add drofus mcp add drofus --scope user ${env_args[@]+"${env_args[@]}"} "${tail[@]}" \
    @@ mcp add drofus --scope user ${env_args[@]+"${env_args[@]}"} "${tail[@]}"
  echo "The API key is stored in $where. Claude Code's configuration does not hold it."
  exit 0
fi

if [ -n "$URL" ]; then
  if [ "$ASK_TOKEN" = 1 ] && [ -z "$TOKEN" ]; then read -r -s -p "Shared secret (not shown): " TOKEN; echo; fi
  if [ -n "$TOKEN" ]; then
    add roommate mcp add --transport http --scope user roommate "$URL" --header "Authorization: Bearer ***" \
      @@ mcp add --transport http --scope user roommate "$URL" --header "Authorization: Bearer $TOKEN"
    echo "Claude Code stores this header in its configuration file in plain text."
  else
    add roommate mcp add --transport http --scope user roommate "$URL" \
      @@ mcp add --transport http --scope user roommate "$URL"
  fi
  exit 0
fi

[ -x "$APP/mcp" ] || { echo "mcp was not found beside this script ($APP)." >&2; exit 1; }
args=(mcp add roommate --scope user -- "$APP/mcp" --server-settings "$SETTINGS/server.toml" --project-settings "$SETTINGS/projects")
add roommate "${args[@]}" @@ "${args[@]}"

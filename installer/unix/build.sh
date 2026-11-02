#!/usr/bin/env bash
# Builds the Mac/Linux package: one tarball with the server, both MCP servers, the viewer and the
# installer script. NO pyRevit toolbar and nothing for Revit -- the producer is Windows-only, and
# this package is for a machine that serves and reads (a shared server, a Mac running Claude Code).
#
# Usage: installer/unix/build.sh [version]     (default: the crate version)
# Output: target/unix-package/roommate-<version>-<os>-<arch>.tar.gz (+ .sha256)
#
# Release binaries are built here, on the platform they are for: they come from CI, never from a
# dev machine someone hands around (the same rule as the Windows installer).
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
cd "$REPO"

VERSION=${1:-$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)}
VERSION=${VERSION#v}
case "$(uname -s)" in Darwin) OS=macos ;; Linux) OS=linux ;; *) echo "unsupported OS: $(uname -s)" >&2; exit 1 ;; esac
case "$(uname -m)" in x86_64|amd64) ARCH=x86_64 ;; arm64|aarch64) ARCH=arm64 ;; *) echo "unsupported CPU: $(uname -m)" >&2; exit 1 ;; esac
NAME=roommate-$VERSION-$OS-$ARCH

[ -f static/vendor/renderer.bundle.js ] || { echo "static/vendor/renderer.bundle.js is missing -- run 'npm run build'." >&2; exit 1; }

echo "Building $NAME ..."
cargo build --release --bin roommate --bin mcp --bin drofus-mcp

OUT=target/unix-package
STAGE=$OUT/$NAME
rm -rf "$STAGE"
mkdir -p "$STAGE/settings-template/projects"
for b in roommate mcp drofus-mcp; do cp "target/release/$b" "$STAGE/$b"; done
# The viewer sits beside the binaries: the server looks for static/ next to itself.
cp -R static "$STAGE/static"
cp installer/unix/roommate.sh installer/unix/register-claude-mcp.sh installer/unix/install.sh installer/unix/README.txt "$STAGE/"
# The shared template, with the Windows paths in its comments made true here.
sed -e 's|%LOCALAPPDATA%\\RoomMate|~/.local/share/roommate|g' -e 's|\\data\\snapshots|/data/snapshots|g' \
  installer/server.toml > "$STAGE/settings-template/server.toml"
cp settings/projects/sample-project.toml "$STAGE/settings-template/projects/"
chmod +x "$STAGE"/*.sh

tar -C "$OUT" -czf "$OUT/$NAME.tar.gz" "$NAME"
( cd "$OUT" && { shasum -a 256 "$NAME.tar.gz" 2>/dev/null || sha256sum "$NAME.tar.gz"; } > "$NAME.tar.gz.sha256" )
echo "Built $OUT/$NAME.tar.gz"

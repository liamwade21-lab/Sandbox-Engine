#!/usr/bin/env bash
# Makes the players' zip: `Sandbox Engine/` holding only the exe (its own files are built in, see
# CLAUDE.md "What players get"), zipped to OUT_DIR/Sandbox-Engine-VERSION-windows.zip.
# Usage: bash .audit/tools/package_release.sh VERSION OUT_DIR   (OUT_DIR outside the project)
set -euo pipefail
VERSION="${1:?version, e.g. 1.0.1}"
OUT="${2:?output folder outside the project}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EXE="$ROOT/target/release/spaceengine-rs.exe"
[ -f "$EXE" ] || { echo "build first: cargo build --release"; exit 1; }
STAGE="$OUT/stage-$VERSION"
rm -rf "$STAGE"
mkdir -p "$STAGE/Sandbox Engine"
cp "$EXE" "$STAGE/Sandbox Engine/spaceengine-rs.exe"
ZIP="$OUT/Sandbox-Engine-$VERSION-windows.zip"
rm -f "$ZIP"
python -I -c "import sys, zipfile; z = zipfile.ZipFile(sys.argv[1], 'w', zipfile.ZIP_DEFLATED, compresslevel=9); z.write(sys.argv[2], 'Sandbox Engine/spaceengine-rs.exe'); z.close()" "$ZIP" "$STAGE/Sandbox Engine/spaceengine-rs.exe"
rm -rf "$STAGE"
ls -la "$ZIP"

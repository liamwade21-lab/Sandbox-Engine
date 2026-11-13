#!/usr/bin/env bash
# Makes the players' zip: `Sandbox Engine/` holding the exe and its `assets` folder beside it (the port's shaders,
# textures, models, Megaton Rainfall's copy, the mods and Garry's Mod's pack; see CLAUDE.md "What players get"), zipped to
# OUT_DIR/Sandbox-Engine-VERSION-windows.zip.
# Usage: bash .audit/tools/package_release.sh VERSION OUT_DIR   (OUT_DIR outside the project)
set -euo pipefail
VERSION="${1:?version, e.g. 1.0.1}"
OUT="${2:?output folder outside the project}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EXE="$ROOT/target/release/spaceengine-rs.exe"
[ -f "$EXE" ] || { echo "build first: cargo build --release"; exit 1; }
ZIP="$OUT/Sandbox-Engine-$VERSION-windows.zip"
mkdir -p "$OUT"
rm -f "$ZIP"
# Python's zipfile (PowerShell's Compress-Archive writes backslashes in the paths). The pack and other already-compressed
# files are stored, the rest deflated.
python -I - "$ZIP" "$EXE" "$ROOT/crates/se_engine/assets" <<'PY'
import os, sys, zipfile
zip_path, exe, assets = sys.argv[1:4]
parts = ["shaders", "textures", "models", "mr", "mods", "gmod"]
stored = {".pak", ".ogg", ".jpg", ".png"}
def add(z, full):
    rel = os.path.relpath(full, assets).replace(os.sep, "/")
    kind = zipfile.ZIP_STORED if os.path.splitext(full)[1].lower() in stored else zipfile.ZIP_DEFLATED
    z.write(full, f"Sandbox Engine/assets/{rel}", compress_type=kind)
with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    z.write(exe, "Sandbox Engine/spaceengine-rs.exe")
    for part in parts:
        top = os.path.join(assets, part)
        if os.path.isfile(top):
            add(z, top)
        elif os.path.isdir(top):
            for d, _, files in os.walk(top):
                for f in sorted(files):
                    add(z, os.path.join(d, f))
        else:
            sys.exit(f"missing: {top}")
PY
ls -la "$ZIP"

#!/bin/bash
# Re-checks this copy against Ruffle itself (UPSTREAM.md): takes the same paths from a clone of
# https://github.com/ruffle-rs/ruffle at the pinned commit, applies patches/*.patch in order, and
# compares the result with this folder. Prints nothing but "identical" when all is well.
#
#   bash vendor/ruffle/check_upstream.sh PATH_TO_A_RUFFLE_CLONE
#
# Not compared: what is ours alone (UPSTREAM.md, patches/, this script), core/prebuilt/ (made by
# remake_playerglobal.sh) and Cargo.lock (cargo writes it for the trimmed workspace).
set -e
COMMIT=4f19836dc657fb1b72a5c72f3433340f75b84050
CLONE="${1:?a Ruffle clone}"
HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
git -c core.autocrlf=false -C "$CLONE" archive --format=tar "$COMMIT" -- \
  Cargo.toml Cargo.lock LICENSE.md README.md clippy.toml rustfmt.toml core render/Cargo.toml render/src \
  render/wgpu render/naga-agal render/naga-pixelbender render/pixel_bender video/Cargo.toml video/src flv swf \
  wstr tools/asc ':(exclude)render/naga-agal/tests' ':(exclude)render/naga-agal/agal-helper' \
  ':(exclude)render/pixel_bender/assembly_tests' ':(exclude)swf/tests' ':(exclude)swf/examples' \
  | tar -x -C "$WORK"
for p in "$HERE"/patches/*.patch; do
  git -c core.autocrlf=false -C "$WORK" apply --whitespace=nowarn "$p"
done
diff -r -q --exclude=UPSTREAM.md --exclude=patches --exclude=check_upstream.sh --exclude=prebuilt \
  --exclude=Cargo.lock --exclude=target "$WORK" "$HERE" && echo identical

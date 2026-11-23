#!/bin/bash
# Compiles the bridge's ActionScript (elitegfx/*.as) into bridge.abc, and the test block
# (proof/Proof.as) into proof.abc, with Ruffle's own compiler
# (vendor/ruffle/tools/asc/asc.jar, against Ruffle's built-ins), then writes bridge.stamp, the
# stamp of the sources the block was made from (a test fails when they differ). A developer step:
# a normal build never needs Java.
#
#   bash crates/elite-gfx/as3/build.sh
#
# Java: JAVA_HOME if set, else the JDK 21 at C:\Users\liamw\tools\jdk-21 (for this script only).
set -e
cd "$(dirname "$0")"
export JAVA_HOME="${JAVA_HOME:-/c/Users/liamw/tools/jdk-21}"
export PATH="$JAVA_HOME/bin:$PATH"
VENDOR=../../../vendor/ruffle
OUT=../../../target/elite-gfx-as3
mkdir -p "$OUT"
# Bridge.as last: its script is the block's entry, run when the movie loads.
java -classpath "$VENDOR/tools/asc/asc.jar" macromedia.asc.embedding.ScriptCompiler -optimize \
  -import "$VENDOR/core/prebuilt/playerglobal_import.abc" -outdir "$OUT" -out bridge \
  elitegfx/Manager.as elitegfx/Store.as elitegfx/Tweener.as elitegfx/Bridge.as > "$OUT/asc.log" 2>&1 \
  || { tail -30 "$OUT/asc.log"; exit 1; }
grep -iE "error|warning" "$OUT/asc.log" | head -20 || true
cp "$OUT/bridge.abc" bridge.abc
# The test block (src/tests/bridge_proof.rs): it drives the bridge, so it is compiled against it.
java -classpath "$VENDOR/tools/asc/asc.jar" macromedia.asc.embedding.ScriptCompiler -optimize \
  -import "$VENDOR/core/prebuilt/playerglobal_import.abc" -import bridge.abc -outdir "$OUT" -out proof \
  proof/Proof.as > "$OUT/asc_proof.log" 2>&1 || { tail -30 "$OUT/asc_proof.log"; exit 1; }
grep -iE "error|warning" "$OUT/asc_proof.log" | head -20 || true
cp "$OUT/proof.abc" proof.abc
cd ../../..
ELITE_GFX_WRITE_STAMP=1 cargo test --quiet --profile agent -p elite-gfx --lib write_bridge_stamp -- --ignored > /dev/null
ls -l crates/elite-gfx/as3/bridge.abc crates/elite-gfx/as3/proof.abc crates/elite-gfx/as3/bridge.stamp

#!/bin/bash
# Sandbox Engine patch c (UPSTREAM.md): remakes core/prebuilt/ (Ruffle's built-in ActionScript,
# compiled) with Ruffle's own compiler, tools/asc/asc.jar, which needs Java. Run it after changing
# any .as file under core/src/avm1/globals or core/src/avm2/globals (our patch b adds some), or
# after taking a newer Ruffle; a normal build refuses a stale core/prebuilt.
#
#   bash vendor/ruffle/remake_playerglobal.sh
#
# Java: JAVA_HOME if set, else the JDK 21 at C:\Users\liamw\tools\jdk-21 (for this script only).
set -e
cd "$(dirname "$0")"
export JAVA_HOME="${JAVA_HOME:-/c/Users/liamw/tools/jdk-21}"
export PATH="$JAVA_HOME/bin:$PATH"
java -version > /dev/null 2>&1 || { echo "no Java at $JAVA_HOME"; exit 1; }
cargo run --quiet --release --target-dir ../../target/vendor-ruffle -p build_playerglobal -- compile core/prebuilt
ls -l core/prebuilt

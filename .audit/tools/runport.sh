#!/usr/bin/env bash
# Run the port once with a script and a final screenshot, one instance at a time across agents.
# Usage: bash .audit/tools/runport.sh OUT.png FRAMES "dev; commands" [extra args...]
REPO="/c/Users/liamw/Desktop/Sandbox Engine"
LOCK="$REPO/.audit/port.lock"
OUT="$1"; FRAMES="$2"; CMDS="$3"; shift 3
# a lock older than 10 minutes is left over from a killed run
for i in $(seq 1 1200); do
  if mkdir "$LOCK" 2>/dev/null; then break; fi
  if [ -n "$(find "$LOCK" -maxdepth 0 -mmin +10 2>/dev/null)" ]; then rmdir "$LOCK" 2>/dev/null; fi
  sleep 2
done
trap 'rmdir "$LOCK" 2>/dev/null' EXIT
cd "$REPO" || exit 1
mkdir -p "$(dirname "$OUT")"
# The port exits a few frames after its screenshot once the file exists: an old one would stay.
rm -f "$OUT"
NO_COLOR=1 RUST_LOG_STYLE=never SE_NO_MUSIC=1 timeout 400 "${PORT_BIN:-target/release/spaceengine-rs.exe}" --run "$CMDS" --screenshot "$OUT" --frames "$FRAMES" "$@" > "$OUT.log" 2>&1
echo "exit $?; log: $OUT.log"
grep -iE "error|panic|warn" "$OUT.log" | grep -v "wgpu_hal" | tail -8
tail -5 "$OUT.log"

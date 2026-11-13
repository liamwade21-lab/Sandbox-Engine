#!/usr/bin/env bash
# F9's table against the game's real air: he flies level at each speed of a line of the table, and the log says what his skin reached.
#   bash .optimize/bench/hero_heat_table.sh [top]
# Then, for each world, straight up through the top of the boom and back: the log says at what height the barrier's state changes.
REPO="/c/Users/liamw/Desktop/Sandbox Engine"; cd "$REPO" || exit 1
S=.optimize/scratch/run/s7; mkdir -p "$S"; W="C:/Users/liamw/Desktop/Sandbox Engine/$S"
line() { # NAME "select BODY; surface ..." HEIGHT V1 V2 V3 V4 V5
  N="$1"; P="$2"; H="$3"; shift 3
  C="timing frame; date 2026-06-21 12:00:00; $P; wait 300; mods on viltrumite; wait 200; hero air; hero boom $W/$N.csv"
  for v in "$@"; do
    C="$C; hero cruise $v $H; wait 170"
  done
  PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" > /dev/null 2>&1
  grep -E "panicked|ERROR" "$S/$N.png.log" | cut -c1-200 | head -3
  python -I - "$S/$N.csv" "$N" "$@" <<'PY'
import sys, csv
f, name, *speeds = sys.argv[1:]
rows = list(csv.DictReader(open(f)))
want = [800, 2000, 3500, 6000, 10000]
out = []
for v, t in zip(speeds, want):
    v = float(v) * 1.0
    # The last line at this speed: the speed's own, with the height's small correction.
    r = [x for x in rows if abs(float(x['speed']) - v) < 0.02 * v]
    r = r[-1] if r else None
    out.append(f"{v/1e3:g} km/s: skin {float(r['skin_k']):.0f} (table {t}, {100*(float(r['skin_k'])/t-1):+.1f}%) heat {float(r['heat_k']):.0f}, Mach {float(r['mach']):.2f}, {float(r['pressure']):.0f} Pa, {r['boom']}" if r else f"{v}: no row")
print(name, *out, sep="\n  ")
print("  events:", [(x['event'], round(float(x['mach']),2)) for x in rows if x['event']])
PY
}
[ "$1" = top ] || line t_earth0 "select Earth; surface 0 -10 0.5 -2 90" 300 1000 1900 2600 4900 9700
[ "$1" = top ] || line t_earth60 "select Earth; surface 0 -10 60 -2 90" 60000 1100 3400 7100 14600 28900
[ "$1" = top ] || line t_mars0 "select Mars; surface 0 0 0.5 -2 90" 300 900 2300 4800 9800 19500
[ "$1" = top ] || line t_venus0 "select Venus; surface 0 0 0.5 -2 90" 300 200 1400 2000 2800 4900
# Mars at 21 km: under 150 Pa, about 1 mbar's air: no boom, heat by density.
[ "$1" = top ] || line t_thin "select Mars; surface 0 0 21 -2 90" 21000 1100 3500 7400 15200 30000
top() { # NAME "select BODY; surface ..." CLIMB_FRAMES
  N="$1"; P="$2"; F="$3"
  C="timing frame; date 2026-06-21 12:00:00; $P; wait 300; mods on viltrumite; wait 200; hero boom $W/$N.csv; hero aim 0 89; hero keys w+shift 3.5; wait 215; hero keys w 300; wait $F; hero aim 0 -89; hero keys w+shift 3.2; wait 195; hero keys w 300; wait $F"
  PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" > /dev/null 2>&1
  grep -E "panicked|ERROR" "$S/$N.png.log" | cut -c1-200 | head -3
  python -I - "$S/$N.csv" "$N" <<'PY'
import sys, csv
f, name = sys.argv[1:]
rows = list(csv.DictReader(open(f)))
last = None
print(name)
for r in rows:
    if r['boom'] != last or r['event']:
        print(f"  {r['boom']:6} {r['event']:5} at {float(r['height'])/1e3:8.2f} km, {float(r['speed']):8.1f} m/s, Mach {float(r['mach']):6.2f}, {float(r['pressure']):10.2f} Pa, state {r['state']}")
        last = r['boom']
PY
}
top o_earth "select Earth; surface 0 -10 2 0 90" 1500
top o_mars "select Mars; surface 0 0 2 0 90" 700
top o_venus "select Venus; surface 0 0 2 0 90" 4000

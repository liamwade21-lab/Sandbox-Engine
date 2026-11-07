"""Checks a `hero sound trace` CSV against F10's table, worked out here on its own, and prints the levels at each settled speed.
python -I .optimize/bench/hero_sound.py FILE [FILE...]
"""
import csv
import math
import sys


def ramp(x, a, b):
    return min(1.0, max(0.0, (x - a) / (b - a)))


def share(p):
    return 0.25 + 0.75 * (ramp(math.log10(p), 0.0, 2.0) if p > 0 else 0.0)


def table(r):
    q, t, flying, charging, p = float(r['q']), float(r['heat_k']), r['flying'] == '1', r['charging'] == '1', float(r['pressure'])
    lq = math.log10(q) if q > 0 else -1e9
    return [ramp(lq, 1, 6), ramp(t, 2500, 6000), 0.12 * (1 - ramp(lq, 0, 2)) if flying else 0.0, share(p) if charging else 0.0]


LOOPS = ['fastFlight_loop', 'fasterFlight_loop', 'sentry_flightfx_loop', 'flightCharge_loop']

for f in sys.argv[1:]:
    rows = list(csv.DictReader(open(f)))
    print(f'== {f}: {len(rows)} frames')
    worst, unsettled, bad = 0.0, 0, 0
    steps = [0.0] * len(LOOPS)
    for i, r in enumerate(rows):
        want = table(r)
        for j, n in enumerate(LOOPS):
            coded = float(r[n + '_want'])
            # The trace rounds to 4 places, the numbers it's worked out from less; a level's slew is 1 per 0.2 s.
            worst = max(worst, abs(coded - want[j]))
            if abs(coded - want[j]) > 0.01:
                bad += 1
            if i:
                dt = max(float(r['clock']) - float(rows[i - 1]['clock']), 1e-6)
                steps[j] = max(steps[j], abs(float(r[n + '_level']) - float(rows[i - 1][n + '_level'])) / dt)
    print(f'  the code\'s curve against this table: worst {worst:.4f}, frames off by over 0.01: {bad}')
    print('  fastest change of a level, per second (limit 5): ' + ' '.join(f'{s:.2f}' for s in steps))
    # A row wherever speed has settled (under 1 % a third of a second), once per stretch.
    print('   clock state      speed         q   mach  heat_k        p | ' + ' '.join(n[:9].rjust(9) for n in LOOPS) + ' | pitch(fast) | shots')
    last = None
    for i in range(20, len(rows)):
        r = rows[i]
        s, s0 = float(r['speed']), float(rows[i - 20]['speed'])
        steady = abs(s - s0) <= 0.01 * max(s, 1.0)
        key = (r['state'], round(math.log10(max(s, 1.0)) * 4), r['charging'])
        if steady and key != last:
            last = key
            lv = [float(r[n + '_level']) for n in LOOPS]
            print(f"  {float(r['clock']):6.2f} {r['state']:9} {s:8.1f} {float(r['q']):9.3g} {float(r['mach']):6.2f} {float(r['heat_k']):7.0f} {float(r['pressure']):8.2f} | " + ' '.join(f'{v:9.3f}' for v in lv) + f" | {float(r['fastFlight_loop_pitch']):.3f} | {r['shots']}")

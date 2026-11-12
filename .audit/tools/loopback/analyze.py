import sys, struct, math, argparse
from array import array

def read_wav(path):
    b = open(path, 'rb').read()
    if b[:4] != b'RIFF' or b[8:12] != b'WAVE':
        sys.exit('not a RIFF/WAVE file')
    pos, fmt, data = 12, None, None
    while pos + 8 <= len(b):
        cid, sz = b[pos:pos+4], struct.unpack_from('<I', b, pos+4)[0]
        body = b[pos+8:pos+8+sz]
        if cid == b'fmt ':
            tag, ch, rate, _, _, bits = struct.unpack_from('<HHIIHH', body)
            if tag == 0xFFFE and len(body) >= 26:
                tag = struct.unpack_from('<H', body, 24)[0]
            fmt = (tag, ch, rate, bits)
        elif cid == b'data':
            data = body
        pos += 8 + sz + (sz & 1)
    if not fmt or data is None:
        sys.exit('missing fmt or data chunk')
    tag, ch, rate, bits = fmt
    if tag == 3 and bits == 32:
        a = array('f'); n = len(data) // 4 * 4; a.frombytes(data[:n])
        if sys.byteorder == 'big': a.byteswap()
        s = a
    elif tag == 1 and bits == 16:
        a = array('h'); n = len(data) // 2 * 2; a.frombytes(data[:n])
        if sys.byteorder == 'big': a.byteswap()
        s = [x / 32768.0 for x in a]
    else:
        sys.exit(f'unsupported format tag={tag} bits={bits}')
    frames = len(s) // ch
    chans = [list(s[c:frames*ch:ch]) for c in range(min(ch, 2))]
    if len(chans) == 1:
        chans.append(chans[0])
    return rate, ch, frames, chans

def db(x):
    return 20 * math.log10(x) if x > 1e-9 else -999.0

def rms(v):
    return math.sqrt(sum(x*x for x in v) / len(v)) if v else 0.0

def main():
    p = argparse.ArgumentParser()
    p.add_argument('wav'); p.add_argument('--floor', type=float, default=-60)
    p.add_argument('--window', type=float, default=0.05)
    a = p.parse_args()
    rate, ch, frames, c = read_wav(a.wav)
    print(f'{a.wav}: {rate} Hz, {ch} ch, {frames} frames, {frames/rate:.3f} s')
    for i, name in enumerate('LR'):
        v = c[i]
        pk = max((abs(x) for x in v), default=0.0)
        clip = sum(1 for x in v if abs(x) >= 0.999)
        print(f'  {name}: peak {pk:.4f} ({db(pk):.1f} dBFS)  RMS {db(rms(v)):.1f} dBFS  clipped samples {clip}')
    w = max(1, int(a.window * rate))
    act = []
    for s in range(0, frames, w):
        act.append(max(db(rms(c[0][s:s+w])), db(rms(c[1][s:s+w]))) > a.floor)
    gap = max(1, round(0.1 / a.window))
    ev, i, n = [], 0, len(act)
    while i < n:
        if act[i]:
            j = i
            k = i + 1
            while k < n and k <= j + gap:   # gap of < 100 ms joins
                if act[k]: j = k
                k += 1
            ev.append((i, j)); i = j + 1
        else:
            i += 1
    print(f'\nEvents (window {a.window*1000:.0f} ms, floor {a.floor} dB):')
    print(f'{"#":>2} {"start s":>8} {"dur s":>6} {"peakL":>7} {"peakR":>7} {"rmsL dB":>8} {"rmsR dB":>8} {"L-R dB":>7}')
    for n_, (i, j) in enumerate(ev, 1):
        s0, s1 = i * w, min(frames, (j + 1) * w)
        l, r = c[0][s0:s1], c[1][s0:s1]
        rl, rr = db(rms(l)), db(rms(r))
        print(f'{n_:>2} {s0/rate:>8.3f} {(s1-s0)/rate:>6.3f} {max(map(abs,l)):>7.4f} {max(map(abs,r)):>7.4f} {rl:>8.1f} {rr:>8.1f} {rl-rr:>7.1f}')
    if not ev: print('  (none above the floor)')

main()

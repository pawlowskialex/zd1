#!/usr/bin/env python3
"""Compare the ADAU1445 setups two stock MCU apps send their DSP: recordings of every write as `{u16 sub-address BE,
u16 length BE, data}` (the emulator's `--renode 'zd1:twi.dsp Record "FILE"'`, or the extracted
re/mcu/extracted/dsp/download_sequence.bin).

    python3 -I re/scripts/dspcmp.py A.rec B.rec

Prints the writes side by side (sub-address, length, CRC-32), then what differs in the end state: the configuration
registers (0xE000-), the program RAM (0x2000, 5-byte words) and the parameter RAM (0x0000, 400 words of 5.23 on 28
bits; a value near a gain is also given in dB).
"""

import math
import struct
import sys
import zlib

PROG, PROG_LEN, PARAMS = 0x2000, 6804, 400


def writes(path):
    d, i, out = open(path, "rb").read(), 0, []
    while i + 4 <= len(d):
        sub, n = struct.unpack_from(">HH", d, i)
        out.append((sub, d[i + 4 : i + 4 + n]))
        i += 4 + n
    return out


def state(ws):
    m, seen = bytearray(0x10000), set()
    for sub, data in ws:
        m[sub : sub + len(data)] = data
        seen.update(range(sub, sub + len(data)))
    return m, seen


def q523(w):
    v = w & 0x0FFFFFFF
    return (v - 0x10000000 if v & 0x08000000 else v) / (1 << 23)


def show(v):
    db = " (%+.1f dB)" % (20 * math.log10(v)) if 0.01 < v < 16 else ""
    return "%+10.6f%s" % (v, db)


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    a, b = writes(sys.argv[1]), writes(sys.argv[2])
    print("# writes: %d and %d" % (len(a), len(b)))
    for i in range(max(len(a), len(b))):
        row = [
            "%04x %5d %08x" % (w[0], len(w[1]), zlib.crc32(w[1])) if w else "-"
            for w in (a[i] if i < len(a) else None, b[i] if i < len(b) else None)
        ]
        print("%3d %-4s %s | %s" % (i, "" if row[0] == row[1] else "DIFF", *row))
    (ma, sa), (mb, sb) = state(a), state(b)
    print("\n# configuration registers that differ (16-bit words; -- = not written)")
    for r in sorted({k & ~1 for k in sa | sb if k >= 0xE000}):
        va = ma[r : r + 2].hex() if {r, r + 1} & sa else "--"
        vb = mb[r : r + 2].hex() if {r, r + 1} & sb else "--"
        if va != vb:
            print("  0x%04x: %s  %s" % (r, va, vb))
    pa, pb = ma[PROG : PROG + PROG_LEN], mb[PROG : PROG + PROG_LEN]
    diff = [i // 5 for i in range(0, PROG_LEN, 5) if pa[i : i + 5] != pb[i : i + 5]]
    print("\n# program RAM: %d of %d words differ" % (len(diff), PROG_LEN // 5))
    print("\n# parameter RAM that differs")
    for p in range(PARAMS):
        wa, wb = struct.unpack_from(">I", ma, 4 * p)[0], struct.unpack_from(">I", mb, 4 * p)[0]
        if wa != wb:
            print("  0x%03x: %08x %s   %08x %s" % (p, wa, show(q523(wa)), wb, show(q523(wb))))


if __name__ == "__main__":
    main()

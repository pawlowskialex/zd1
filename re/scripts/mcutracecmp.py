#!/usr/bin/env python3
"""What an MCU app does with the peripherals, from a zemu MCU trace (emu/run/NAME/mcu_trace.txt): the pins (each one's
states in order), the ADC channels enabled and the values read, the peripheral registers written, the I2C devices, the
CS2000 and the LED. With two traces, only what differs.

    python3 -I re/scripts/mcutracecmp.py TRACE [TRACE2]

To trace a stock app whole, without the USB block's polling and past the default line cap:
    zemu run MODULE.bcd --nand --mcu FLASH --renode 'zd1:usbb TraceAccesses false' \
        --renode 'zd1:pm TraceMaxLines 5000000' --at-once --sleep 60 --allow-panic --name NAME
"""

import re
import sys
from collections import OrderedDict, defaultdict

ADC = 0xFFFF3C00


def summary(path):
    pins = OrderedDict()
    adc_en, adc_vals = set(), defaultdict(set)
    writes = defaultdict(set)  # (block, offset) -> values
    i2c, cs = defaultdict(set), set()
    leds = []
    for line in open(path, errors="replace"):
        m = re.match(r"\s*([\d.]+)\s+(R|W) ([0-9a-f]{8}) (?:=|<-) ([0-9a-f]{8})\s+(\S+)", line)
        if m:
            rw, a, v, blk = m.group(2), int(m.group(3), 16), int(m.group(4), 16), m.group(5)
            if rw == "W" and blk not in ("GPIO",):
                writes[(blk, a)].add(v)
            if rw == "W" and a == ADC + 0x10:
                adc_en |= {c for c in range(8) if v >> c & 1}
            if rw == "R" and ADC + 0x30 <= a < ADC + 0x50:
                adc_vals[(a - ADC - 0x30) // 4].add(v)
            continue
        m = re.match(r"\s*[\d.]+\s+PIN (P[AB]\d\d) (\S+) -> (\S+)", line)
        if m:
            p = pins.setdefault(m.group(1), [])
            t = m.group(3)
            if not p or p[-1] != t:
                p.append(t)
            continue
        m = re.match(r"\s*[\d.]+\s+I2C (\S+)\s+W (\S+)", line)
        if m:
            i2c[m.group(1)].add(m.group(2))
            continue
        m = re.match(r"\s*[\d.]+\s+CS2000 W reg (\S+) <- (\S+)", line)
        if m:
            cs.add((m.group(1), m.group(2)))
            continue
        m = re.match(r"\s*[\d.]+\s+LED rgb\s+(\d+)\s+(\d+)\s+(\d+)", line)
        if m and (not leds or leds[-1] != m.groups()):
            leds.append(m.groups())
    return dict(pins=pins, adc_en=adc_en, adc_vals=adc_vals, writes=writes, i2c=i2c, cs=cs, leds=leds)


def pin_line(states):
    s = states if len(states) <= 6 else states[:3] + ["... %d changes ..." % (len(states) - 4)] + states[-1:]
    return " > ".join(s)


def report(name, s):
    print("## %s" % name)
    print("pins (%d):" % len(s["pins"]))
    for p in sorted(s["pins"]):
        print("  %s: %s" % (p, pin_line(s["pins"][p])))
    print("ADC enabled: %s" % sorted(s["adc_en"]))
    for c in sorted(s["adc_vals"]):
        v = sorted(s["adc_vals"][c])
        print(
            "  ch%d read: %s"
            % (c, ", ".join("0x%x" % x for x in v[:6]) + (" ... (%d values)" % len(v) if len(v) > 6 else ""))
        )
    print("I2C: " + "; ".join("%s %d regs" % (k, len(v)) for k, v in s["i2c"].items()))
    blocks = defaultdict(set)
    for b, a in s["writes"]:
        blocks[b].add(a)
    print("registers written: " + ", ".join("%s %d" % (b, len(v)) for b, v in sorted(blocks.items())))


def diff(na, a, nb, b):
    print("## %s vs %s" % (na, nb))
    for p in sorted(set(a["pins"]) | set(b["pins"])):
        x, y = a["pins"].get(p), b["pins"].get(p)
        if x != y:
            print("  pin %s: %s | %s" % (p, pin_line(x) if x else "untouched", pin_line(y) if y else "untouched"))
    if a["adc_en"] != b["adc_en"]:
        print("  ADC enabled: %s | %s" % (sorted(a["adc_en"]), sorted(b["adc_en"])))
    for c in sorted(set(a["adc_vals"]) | set(b["adc_vals"])):
        x, y = a["adc_vals"].get(c, set()), b["adc_vals"].get(c, set())
        if (min(x, default=None), max(x, default=None)) != (min(y, default=None), max(y, default=None)):
            r = lambda s: "0x%x-0x%x (%d)" % (min(s), max(s), len(s)) if s else "-"
            print("  ADC ch%d: %s | %s" % (c, r(x), r(y)))
    for k in sorted(set(a["writes"]) | set(b["writes"])):
        x, y = a["writes"].get(k), b["writes"].get(k)
        if x != y and not (k[0] in ("SPI0", "USART") and x and y):
            f = lambda s: ",".join("%x" % v for v in sorted(s)[:4]) + ("..." if len(s) > 4 else "") if s else "never"
            print("  %s %08x: %s | %s" % (k[0], k[1], f(x), f(y)))
    for dev in sorted(set(a["i2c"]) | set(b["i2c"])):
        if a["i2c"].get(dev) != b["i2c"].get(dev):
            print("  I2C %s: %s | %s" % (dev, sorted(a["i2c"].get(dev, []))[:12], sorted(b["i2c"].get(dev, []))[:12]))
    if a["cs"] != b["cs"]:
        print("  CS2000: %s | %s" % (sorted(a["cs"]), sorted(b["cs"])))
    if a["leds"][:12] != b["leds"][:12]:
        print("  LED: %s | %s" % (a["leds"][:12], b["leds"][:12]))


if __name__ == "__main__":
    s = [summary(p) for p in sys.argv[1:]]
    if len(s) == 1:
        report(sys.argv[1], s[0])
    else:
        diff(sys.argv[1], s[0], sys.argv[2], s[1])

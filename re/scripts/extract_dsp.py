#!/usr/bin/env python3
"""Extract the ADAU1445 download sequence (SigmaStudio's default: program, configuration, parameters, start) from a
stock MCU app, as its download function writes it.

    python3 -I re/scripts/extract_dsp.py [--model zd1|a7] IMAGE OUTDIR    -> download_sequence.{bin,txt,json}

IMAGE: re/gen/mcu-app/image.bin (ZD1 2.04.05: FUN_80020980) or re/gen/mcu-app-a7/image.bin (A7 2.04.77:
FUN_8001ff4c), from `re/ghidra/setup.py`. Each write's data is either initialised RAM (.data, base R6) or a zeroed
buffer (.bss, base R5: all zeros at boot). The A7's download is the same 24 writes with its own data, two of them
narrowed (0xE101 and 0xE141: 2 bytes instead of 0xE100 and 0xE140's 4) and 0xE240 split in two (4 bytes, then 12 at
0xE247), and the start's 0xE220 / 0xE228 in the other order.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from avr32img import Image  # noqa: E402

# (base, offset, sub-address, length); base "R6" = .data at R6, "R5" = .bss (zeros), None = an empty write
MODELS = {
    "zd1": (
        0xC54,
        [
            ("R6", 0x13C, 0xE220, 2),
            ("R5", 0x13C, 0xE228, 2),
            ("R6", 0x140, 0x2000, 0x1A94),
            ("R6", 0x1BD4, 0xE000, 0x12),
            ("R6", 0x1BE8, 0xE040, 0x12),
            ("R5", 0x140, 0xE049, 2),
            ("R6", 0x1BFC, 0xE080, 0x10),
            ("R6", 0x1C0C, 0xE088, 0x10),
            ("R6", 0x1C1C, 0xE090, 0x18),
            ("R5", 0x144, 0xE0C0, 0x18),
            ("R6", 0x1C34, 0xE100, 4),
            ("R6", 0x1C38, 0xE140, 4),
            ("R5", 0x15C, 0xE180, 0x30),
            ("R5", 0x18C, 0xE1C0, 0x40),
            ("R6", 0x1C3C, 0xE200, 0x54),
            ("R6", 0x1C90, 0xE240, 0x1A),
            ("R6", 0x1CAC, 0xE280, 2),
            ("R6", 0x1CB0, 0x0000, 0x640),
            ("R6", 0x22F0, 0xE21F, 2),
            (None, 0, 0x6000, 0),
            ("R6", 0x22F4, 0xE228, 2),
            ("R6", 0x22F8, 0xE220, 2),
            ("R5", 0x1CC, 0xE221, 2),
            ("R6", 0x22FC, 0xE221, 2),
        ],
    ),
    "a7": (
        0x6AC,
        [
            ("R6", 0x13C, 0xE220, 2),
            ("R5", 0x13C, 0xE228, 2),
            ("R6", 0x140, 0x2000, 0x1A94),
            ("R6", 0x1BD4, 0xE000, 0x12),
            ("R6", 0x1BE8, 0xE040, 0x12),
            ("R5", 0x140, 0xE049, 2),
            ("R6", 0x1BFC, 0xE080, 0x10),
            ("R6", 0x1C0C, 0xE088, 0x10),
            ("R6", 0x1C1C, 0xE090, 0x18),
            ("R5", 0x144, 0xE0C0, 0x18),
            ("R5", 0x15C, 0xE101, 2),
            ("R6", 0x1C34, 0xE141, 2),
            ("R5", 0x160, 0xE180, 0x30),
            ("R5", 0x190, 0xE1C0, 0x40),
            ("R6", 0x1C38, 0xE200, 0x54),
            ("R5", 0x1D0, 0xE240, 4),
            ("R5", 0x1D4, 0xE247, 0xC),
            ("R6", 0x1C8C, 0xE280, 2),
            ("R6", 0x1C90, 0x0000, 0x640),
            ("R6", 0x22D0, 0xE21F, 2),
            (None, 0, 0x6000, 0),
            ("R6", 0x22D4, 0xE220, 2),
            ("R6", 0x22D8, 0xE228, 2),
            ("R5", 0x1E0, 0xE221, 2),
            ("R6", 0x22DC, 0xE221, 2),
        ],
    ),
}


def main():
    # no argparse: it imports inspect, which imports dis, and this directory's dis.py comes first on sys.path
    args, model = sys.argv[1:], "zd1"
    if args[:1] == ["--model"] and len(args) > 1:
        model, args = args[1], args[2:]
    if model not in MODELS or len(args) != 2:
        sys.exit(__doc__)
    img = Image(args[0])
    r6, seq = MODELS[model]
    outdir = args[1]
    os.makedirs(outdir, exist_ok=True)
    txt, allbin, meta = [], bytearray(), []
    for i, (base, off, addr, n) in enumerate(seq):
        data = img.bytes(r6 + off, n) if base == "R6" else bytes(n)  # R5 arrays live in .bss -> all zero
        if len(data) != n:
            sys.exit("step %d: .data+0x%x is outside the image's .data" % (i, r6 + off))
        allbin += addr.to_bytes(2, "big") + n.to_bytes(2, "big") + data
        src = (".data+" + hex(r6 + off)) if base == "R6" else (".bss(zero)" if base else "none")
        meta.append(dict(step=i, i2c_addr7=0x38, subaddr=hex(addr), length=n, src=src))
        txt.append(f"{i:02d} write subaddr=0x{addr:04x} len={n:5d} {data[:24].hex(' ')}{' ...' if n > 24 else ''}")
    open(os.path.join(outdir, "download_sequence.txt"), "w").write("\n".join(txt) + "\n")
    open(os.path.join(outdir, "download_sequence.bin"), "wb").write(allbin)
    json.dump(meta, open(os.path.join(outdir, "download_sequence.json"), "w"), indent=1)
    print("\n".join(txt))


if __name__ == "__main__":
    main()

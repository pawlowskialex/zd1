#!/usr/bin/env python3
"""Patch the stock Zeppelin Air MCU firmware with the telnet hook (README.md): once the module is ready, the MCU
writes reg 0x75 = "23" and reg 0x76 = 5, the module's KnOS shell on telnet port 23, re-asserted every ~30 s. Python
only: the hook blob is prebuilt (telnet_hook.bin; build.sh rebuilds it from telnet_hook.S).

  usage: python3 -I stock-patch/patch.py OUT.hex [--stock STOCK.hex]   # ZD1 2.04.05, a .hex for DFU
         python3 -I stock-patch/patch.py --fw IN.fw OUT.fw             # ZD1L 2.04.35 (or 2.01.04), a B&W .fw

The .hex path flashes over DFU: zd1 bootloader, then zd1 dfu write OUT.hex --backup BACKUP.tar --yes --start (a
`zd1 dfu backup` first); back to stock, the same with blobs/mcu/zd1-2.04.05.hex. The --fw path repacks a B&W over-the-air
.fw (bwfw.py) with its MCU app patched and its module .bcd untouched, for a unit whose only MCU-flash route is the .fw
(the Lightning model: docs/mcu/stock-firmware.md); what on that unit takes a .fw's MCU part is unconfirmed.

The hook's three patch sites and every address the blob uses are byte-identical in ZD1 2.04.05 and ZD1L 2.04.35 but for
sbrk's heap-base literal, which the -4 data shift moves (0x8002AE28 -> 0x8002AE24); patch() locates it instead of
hardcoding it. The anchors are asserted, so a build that moved them (the A7's) fails loudly rather than flashing wrong.
Our own firmware (mcu/) puts the shell on telnet with `zd1 reg write` (docs/module/knos.md).
"""

import argparse
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "tools"))
from zeppelin.fw import bwfw, ihex  # noqa: E402

BASE = 0x80000000
CAVE = 0x80038100  # free flash the hook goes to (telnet_hook.S is linked there, hook.ld)
VOTASK_PTR = 0x8001E870  # the literal the BcoWrapper tick calls voTask through
BSS_END_LIT = 0x80008088  # crt0's bss-clear end
OLD_VOTASK = 0x8000E19C
OLD_MEMEND = 0x8250
NEW_MEMEND = 0x8258  # 0x8250..0x8257 become the hook's state (crt0 zeroes it)
HEAP_LIT_RANGE = (0x8002A000, 0x8002B000)  # sbrk's heap-base literal (== OLD_MEMEND) sits here in both builds


def heap_base_lit(img):
    """The address of sbrk's heap-base literal: the unique word == OLD_MEMEND in the sbrk range (the other two, at
    BSS_END_LIT and elsewhere, are outside it)."""
    hits = [
        BASE + i
        for i in range(*(a - BASE for a in HEAP_LIT_RANGE))
        if struct.unpack_from(">I", img, i)[0] == OLD_MEMEND
    ]
    if len(hits) != 1:
        raise ValueError("heap-base literal not unique: %r" % [hex(h) for h in hits])
    return hits[0]


def patch(stock, blob):
    """The stock image with the hook at CAVE, voTask's call pointer on it and 8 bytes of RAM carved for its state."""
    rd = lambda a: struct.unpack_from(">I", stock, a - BASE)[0]  # noqa: E731
    assert rd(VOTASK_PTR) == OLD_VOTASK, "voTask pointer is not stock"
    assert rd(BSS_END_LIT) == OLD_MEMEND, "bss-end literal is not stock"
    heap = heap_base_lit(stock)
    assert CAVE - BASE + len(blob) <= 0x40000, "the hook runs past the app region"
    img = bytearray(stock)
    end = CAVE - BASE + len(blob)
    if len(img) < end:
        img += b"\xff" * (end - len(img))
    assert img[CAVE - BASE : end] == b"\xff" * len(blob), "the cave is not free"
    img[CAVE - BASE : end] = blob
    for addr, val in ((VOTASK_PTR, CAVE), (BSS_END_LIT, NEW_MEMEND), (heap, NEW_MEMEND)):
        struct.pack_into(">I", img, addr - BASE, val)
    return img, heap


def patch_fw(fw, blob):
    """A B&W .fw repacked with its MCU .dmb patched (the module .bcd and the release unchanged)."""
    w, bcd, mcu, dmb = bwfw.parse(fw)
    if not dmb:
        raise ValueError("the .fw's MCU part is not a .dmb")
    name, app = dmb
    built = struct.unpack_from(">I", mcu, 4)[0]
    version = mcu[24 + bwfw.DMB_NAME : 24 + bwfw.DMB_NAME + 3]
    img, heap = patch(bwfw.flash(app), blob)
    new_app = bytes(img[bwfw.APP : CAVE - BASE + len(blob)])  # 0x80008000 .. hook end
    return bwfw.wrap(bcd, bwfw.make_dmb(new_app, name, version, built), w[5]), heap, name


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out", nargs="?", metavar="OUT.hex")
    ap.add_argument("--stock", default=os.path.join(HERE, "..", "blobs", "mcu", "zd1-2.04.05.hex"))
    ap.add_argument("--fw", nargs=2, metavar=("IN.fw", "OUT.fw"), help="patch a B&W .fw's MCU app (ZD1L)")
    a = ap.parse_args()
    if not a.fw and not a.out:
        ap.error("OUT.hex is required (or use --fw IN.fw OUT.fw)")
    blob = open(os.path.join(HERE, "telnet_hook.bin"), "rb").read()
    if a.fw:
        fw_in, fw_out = a.fw
        out, heap, name = patch_fw(open(fw_in, "rb").read(), blob)
        open(fw_out, "wb").write(out)
        print("patched %s in %s; hook %d B at 0x%08x; heap literal 0x%08x" % (name, fw_out, len(blob), CAVE, heap))
        return
    img, heap = patch(ihex.image(open(a.stock).read(), BASE)[1], blob)
    open(a.out, "w").write(ihex.dump(img, BASE))
    print(
        "hook %d bytes at 0x%08x; voTask pointer 0x%08x -> 0x%08x; bss-end/heap(0x%08x) 0x%x -> 0x%x; image end 0x%08x"
        % (len(blob), CAVE, OLD_VOTASK, CAVE, heap, OLD_MEMEND, NEW_MEMEND, BASE + len(img))
    )
    print("wrote", a.out)


if __name__ == "__main__":
    main()

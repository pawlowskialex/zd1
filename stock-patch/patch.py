#!/usr/bin/env python3
"""Patch a stock Zeppelin Air MCU firmware with the telnet hook (README.md): once the module is ready, the MCU
writes reg 0x75 = "23" and reg 0x76 = 5, the module's KnOS shell on telnet port 23, re-asserted every ~30 s. Python
only: the hook blob is prebuilt (telnet_hook.bin; build.sh rebuilds it from telnet_hook.S).

  usage: python3 -I stock-patch/patch.py OUT.hex [--stock STOCK.hex]   # ZD1 2.04.05, a .hex for DFU
         python3 -I stock-patch/patch.py --fw IN.fw OUT.fw             # ZD1L 2.04.35 or A7 2.04.77, a B&W .fw

The .hex path flashes over DFU: zd1 bootloader, then zd1 dfu write OUT.hex --backup BACKUP.tar --yes --start (a
`zd1 dfu backup` first); back to stock, the same with blobs/zd1/mcu/zd1-2.04.05.hex. The --fw path repacks a B&W over-the-air
.fw (bwfw.py) with its MCU app patched and its module .bcd untouched, for a unit whose only MCU-flash route is the .fw
(the Lightning model and the A7: docs/mcu/stock-firmware.md); what on those units takes a .fw's MCU part is unconfirmed.

Three builds are handled, keyed on the crt0 bss-end value (the word at BSS_END_LIT, == the heap base / carved-RAM base):

  memend  build            voTask/WRITEREG   STATUS   ATTACH   notes
  0x8250  ZD1 2.04.05  }    identical         0x3ee0   0x4df4   the hook's three sites are byte-identical; only
  0x8250  ZD1L 2.04.35 }    (0x8000e19c /     0x3ee0   0x4df4   sbrk's heap-base literal moved -4 (data shift),
                            0x8000b910)                          and patch() locates it by value, not address.
  0x7a08  A7 2.04.77       identical         0x3918   0x482c   the RAM data layout shifted: STATUS/ATTACH -0x5c8,
                                                                bss end -0x848, the heap literal moved; voTask's
                                                                call pointer moved (0x8001e870 -> 0x8001de48).

ZD1L 2.01.04 is an older, structurally different build (voTask/WRITEREG are not at the shared addresses); it is not
handled and is refused as an unrecognised build.

voTask and WRITEREG keep the same address in all three, so only four literals in the hook blob are build-specific
(STATUS, ATTACH, and the carved STATE/LASTCOUNT, which follow memend). patch() relocates those four in the committed
blob from its ZD1 sentinels, locates the voTask call pointer and sbrk's heap literal by value, and carves 8 RAM bytes
at memend. An unrecognised build (unknown memend) is refused rather than patched wrong. Our own firmware (mcu/) puts
the shell on telnet with `zd1 reg write` (docs/module/knos.md).
"""

import argparse
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "tools"))
from zeppelin.fw import bwfw, ihex  # noqa: E402

BASE = 0x80000000
CAVE = 0x80038100  # free flash the hook goes to (telnet_hook.S is linked there, hook.ld); free in all handled builds
BSS_END_LIT = 0x80008088  # crt0's bss-clear end literal (same location in all handled builds; its value is `memend`)
OLD_VOTASK = 0x8000E19C  # dmhBcoMgr_voTask, same address in all handled builds
HEAP_LIT_RANGE = (0x8002A000, 0x8002B000)  # sbrk's heap-base literal (== memend) sits here in every handled build

# The committed blob (telnet_hook.bin) is built with ZD1 constants; these four literals are build-specific and get
# relocated for other builds. STATE/LASTCOUNT follow memend; STATUS/ATTACH come from the build profile below.
BLOB_STATUS = 0x3EE0
BLOB_ATTACH = 0x4DF4
BLOB_STATE = 0x8250  # == ZD1 memend
BLOB_LASTCOUNT = 0x8254  # == ZD1 memend + 4

# Keyed on memend (the value at BSS_END_LIT): the module-comm STATUS word and attach-state byte the hook gates on.
# voTask/WRITEREG are identical across builds, so they are not listed here.
PROFILES = {
    0x8250: {"name": "ZD1 2.04.05 / ZD1L 2.04.35", "status": 0x3EE0, "attach": 0x4DF4},
    0x7A08: {"name": "A7 2.04.77", "status": 0x3918, "attach": 0x482C},
}


def unique_word(img, val, lo=0, hi=None, what="word"):
    """The single flash address holding an aligned big-endian word == val in [lo, hi); raises if not exactly one."""
    lo = lo if lo < BASE else lo - BASE
    hi = min(len(img), len(img) if hi is None else hi - BASE)
    hits = [BASE + i for i in range(lo, hi - 3, 4) if struct.unpack_from(">I", img, i)[0] == val]
    if len(hits) != 1:
        raise ValueError("%s not unique (==0x%x): %r" % (what, val, [hex(h) for h in hits]))
    return hits[0]


def relocate_blob(blob, memend, profile):
    """The committed ZD1 blob with its four build-specific literals rewritten for the target build: STATUS/ATTACH from
    the profile, and the carved STATE/LASTCOUNT at memend / memend+4. Offsets are found up front, then written, so a
    rewritten value can never be mistaken for another sentinel."""
    subs = {
        BLOB_STATUS: profile["status"],
        BLOB_ATTACH: profile["attach"],
        BLOB_STATE: memend,
        BLOB_LASTCOUNT: memend + 4,
    }
    out = bytearray(blob)
    writes = []
    for sentinel, value in subs.items():
        off = unique_word(blob, sentinel, what="blob literal 0x%x" % sentinel) - BASE
        writes.append((off, value))
    for off, value in writes:
        struct.pack_into(">I", out, off, value)
    return bytes(out)


def patch(stock, blob):
    """The stock image with the hook at CAVE, voTask's call pointer on it and 8 RAM bytes carved for its state.
    Returns (image, heap-literal addr, build name)."""
    rd = lambda a: struct.unpack_from(">I", stock, a - BASE)[0]  # noqa: E731
    memend = rd(BSS_END_LIT)
    profile = PROFILES.get(memend)
    if profile is None:
        raise ValueError(
            "unrecognised build: memend (bss-end @0x%08x) = 0x%x, not in %r"
            % (BSS_END_LIT, memend, [hex(k) for k in PROFILES])
        )
    new_memend = memend + 8  # [memend, memend+8) become the hook's STATE/LASTCOUNT (crt0 zeroes them)
    votask_ptr = unique_word(stock, OLD_VOTASK, hi=CAVE, what="voTask call pointer")
    heap = unique_word(stock, memend, lo=HEAP_LIT_RANGE[0], hi=HEAP_LIT_RANGE[1], what="heap-base literal")
    blob = relocate_blob(blob, memend, profile)
    assert CAVE - BASE + len(blob) <= 0x40000, "the hook runs past the app region"
    img = bytearray(stock)
    end = CAVE - BASE + len(blob)
    if len(img) < end:
        img += b"\xff" * (end - len(img))
    assert img[CAVE - BASE : end] == b"\xff" * len(blob), "the cave is not free"
    img[CAVE - BASE : end] = blob
    for addr, val in ((votask_ptr, CAVE), (BSS_END_LIT, new_memend), (heap, new_memend)):
        struct.pack_into(">I", img, addr - BASE, val)
    return img, heap, profile["name"]


def patch_fw(fw, blob):
    """A B&W .fw repacked with its MCU .dmb patched (the module .bcd and the release unchanged)."""
    w, bcd, mcu, dmb = bwfw.parse(fw)
    if not dmb:
        raise ValueError("the .fw's MCU part is not a .dmb")
    name, app = dmb
    built = struct.unpack_from(">I", mcu, 4)[0]
    version = mcu[24 + bwfw.DMB_NAME : 24 + bwfw.DMB_NAME + 3]
    img, heap, build = patch(bwfw.flash(app), blob)
    new_app = bytes(img[bwfw.APP : CAVE - BASE + len(blob)])  # 0x80008000 .. hook end
    return bwfw.wrap(bcd, bwfw.make_dmb(new_app, name, version, built), w[5]), heap, name, build


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out", nargs="?", metavar="OUT.hex")
    ap.add_argument("--stock", default=os.path.join(HERE, "..", "blobs", "zd1", "mcu", "zd1-2.04.05.hex"))
    ap.add_argument("--fw", nargs=2, metavar=("IN.fw", "OUT.fw"), help="patch a B&W .fw's MCU app (ZD1L or A7)")
    a = ap.parse_args()
    if not a.fw and not a.out:
        ap.error("OUT.hex is required (or use --fw IN.fw OUT.fw)")
    blob = open(os.path.join(HERE, "telnet_hook.bin"), "rb").read()
    if a.fw:
        fw_in, fw_out = a.fw
        out, heap, name, build = patch_fw(open(fw_in, "rb").read(), blob)
        open(fw_out, "wb").write(out)
        print(
            "patched %s (%s) in %s; hook %d B at 0x%08x; heap literal 0x%08x"
            % (name, build, fw_out, len(blob), CAVE, heap)
        )
        return
    img, heap, build = patch(ihex.image(open(a.stock).read(), BASE)[1], blob)
    open(a.out, "w").write(ihex.dump(img, BASE))
    print(
        "patched %s; hook %d bytes at 0x%08x -> voTask 0x%08x; bss-end/heap literal 0x%08x; image end 0x%08x"
        % (build, len(blob), CAVE, OLD_VOTASK, heap, BASE + len(img))
    )
    print("wrote", a.out)


if __name__ == "__main__":
    main()

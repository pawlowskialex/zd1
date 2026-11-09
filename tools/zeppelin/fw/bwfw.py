#!/usr/bin/env python3
"""A B&W over-the-air .fw (A5, A7, Z2, Zeppelin Air with Lightning; docs/mcu/stock-firmware.md): a 32-byte wrapper, a
.bcd, then the MCU part. Cuts one into its parts for the emulator, or packs one.

    python3 -I tools/zeppelin/fw/bwfw.py FILE.fw [OUTDIR]
    python3 -I tools/zeppelin/fw/bwfw.py --pack OUT.fw BCD APP.bin --name N.dmb --version 2.04.35 [--hwid 0xBE]
    python3 -I tools/zeppelin/fw/bwfw.py --selftest FILE.fw...     # each rebuilt from its parts, byte for byte

OUTDIR gets module.bcd, mcu.bin (the MCU part as it is) and, when the MCU part is an AVR32 .dmb (A7, ZD1L),
mcu_flash.bin: a 256 KB flash image for `zemu run OUTDIR/module.bcd --mcu OUTDIR/mcu_flash.bin` = B&W's ZD1 bootloader,
0x80004000-0x80007FFF from the ZD1's stock app (its jump to 0x80008000; the .dmb starts there) and the .dmb's app.
The bootloader is the ZD1's: the other models' are in no .fw. Emulation only, never flashed.

--pack takes an app linked at 0x80008000 (`nix build .#mcu-dmb`: a .dmb has no room for 0x80004000-0x80007FFF). What
reads a .fw's MCU part on a real speaker has not been seen: a packed file says nothing about whether one would take it.
"""

import argparse
import calendar
import os
import struct
import sys
import time
import zlib

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
from zeppelin import paths  # noqa: E402
from zeppelin.fw import bcd as bcdfmt  # noqa: E402

FLASH = 0x40000
APP = 0x8000  # the .dmb's app goes to 0x80008000
DMB_MAGIC = b"\x00\x01\x00\x80"
DMB_HEADER = 128
DMB_NAME = 20  # the name's field, at +24; the version's three bytes follow it


def crc16_xmodem(data):
    c = 0
    for x in data:
        c ^= x << 8
        for _ in range(8):
            c = ((c << 1) ^ 0x1021) & 0xFFFF if c & 0x8000 else (c << 1) & 0xFFFF
    return c


def make_dmb(app, name, version, built):
    """A .dmb (big-endian): magic, build time, the app's length, its offset in flash, its CRC-16, the name, the version
    as three bytes, then the app at +128."""
    n = name.encode()
    if len(n) >= DMB_NAME:
        raise ValueError("the .dmb's name %r is too long" % name)
    head = DMB_MAGIC + struct.pack(">III", built, len(app), APP) + struct.pack(">H", crc16_xmodem(app))
    head = head.ljust(24, b"\0") + n.ljust(DMB_NAME, b"\0") + bytes(version)
    return head.ljust(DMB_HEADER, b"\0") + app


def wrap(bcd, mcu, version):
    """The wrapper (u32 LE: its version, the .bcd's length, 0 = not encrypted, the MCU part's length, CRC-32 of the two,
    the release as a decimal number, 0, 0), the .bcd, the MCU part and the newline every shipped file ends in."""
    body = bcd + mcu
    return struct.pack("<8I", 2, len(bcd), 0, len(mcu), zlib.crc32(body) & 0xFFFFFFFF, version, 0, 0) + body + b"\n"


def set_hwid(bcd, hwid):
    """The .bcd with its header's HWID replaced (the header's other half and its CRC kept right)."""
    f = bytearray(bcd)
    word = struct.unpack_from("<I", f, 0x20)[0]
    struct.pack_into("<I", f, 0x20, (word & 0xFFFF0000) | hwid)
    struct.pack_into("<I", f, 0x2C, bcdfmt.header_crc(f))
    return bytes(f)


def release(text):
    """ "2.04.35" -> (the three bytes of a .dmb, the wrapper's 20435)."""
    parts = [int(x) for x in text.split(".")]
    if len(parts) != 3 or not all(0 <= x < 100 for x in parts):
        raise ValueError("a version is M.mm.pp: %r" % text)
    return bytes(parts), parts[0] * 10000 + parts[1] * 100 + parts[2]


def bcd_built(bcd):
    """The .bcd's build stamp (14 digits, UTC taken as is) as Unix time: the same input packs to the same file."""
    return calendar.timegm(time.strptime(bcd[8:0x18].decode().strip(), "%Y%m%d%H%M%S"))


def parse(d):
    """-> (wrapper words, the .bcd, the MCU part, the .dmb's name and app or None)."""
    w = struct.unpack_from("<8I", d)
    if w[0] != 2 or w[2] != 0:
        raise ValueError("not an unencrypted wrapper (version %d, encryption mode %d)" % (w[0], w[2]))
    bcd, mcu = d[32 : 32 + w[1]], d[32 + w[1] : 32 + w[1] + w[3]]
    if bcd[:4] != b"bCoD" or len(mcu) != w[3]:
        raise ValueError("no .bcd after the header, or the file is short")
    dmb = None
    if mcu[:4] == DMB_MAGIC:
        n = struct.unpack_from(">I", mcu, 8)[0]
        name = mcu[24 : mcu.index(b"\0", 24)].decode()
        if 128 + n > len(mcu) or APP + n > FLASH:
            raise ValueError("the .dmb's app length 0x%x doesn't fit" % n)
        dmb = (name, mcu[128 : 128 + n])
    return w, bcd, mcu, dmb


def flash(app):
    bl, stock = open(paths.MCU_BOOTLOADER, "rb").read(), paths.stock_mcu_app()
    img = bl + stock[: APP - len(bl)] + app
    return img + b"\xff" * (FLASH - len(img))


def rebuild(d):
    """A shipped .fw built again from its parsed parts."""
    w, bcd, mcu, dmb = parse(d)
    if dmb:
        built = struct.unpack_from(">I", mcu, 4)[0]
        mcu = make_dmb(dmb[1], dmb[0], mcu[24 + DMB_NAME : 24 + DMB_NAME + 3], built)
    return wrap(bcd, mcu, w[5])


def selftest(files):
    bad = 0
    for p in files:
        d = open(p, "rb").read()
        same = rebuild(d) == d
        bad += not same
        print("%s %s" % ("same" if same else "DIFFERENT", os.path.basename(p)))
    sys.exit(1 if bad or not files else 0)


def pack(a):
    out, bcd_path, app_path = a.pack
    bcd = open(bcd_path, "rb").read()
    bcdfmt.sections(bcd)
    if a.hwid is not None:
        bcd = set_hwid(bcd, a.hwid)
    app = open(app_path, "rb").read()
    if APP + len(app) > 0x3FE00:
        raise ValueError("the app is 0x%x B: it runs into the boot log's page" % len(app))
    if not a.name or not a.version:
        raise ValueError("--pack needs --name and --version")
    bytes3, number = release(a.version)
    open(out, "wb").write(wrap(bcd, make_dmb(app, a.name, bytes3, bcd_built(bcd)), number))
    print("%s: .bcd %d B, %s %d B at 0x80008000, version %s" % (out, len(bcd), a.name, len(app), a.version))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("fw", nargs="*", metavar="FILE.fw [OUTDIR]")
    ap.add_argument("--pack", nargs=3, metavar=("OUT.fw", "BCD", "APP.bin"))
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--name", help="the .dmb's name")
    ap.add_argument("--version", help="M.mm.pp")
    ap.add_argument("--hwid", type=lambda x: int(x, 0), help="stamp this HWID into the .bcd's header")
    a = ap.parse_args()
    if a.pack:
        return pack(a)
    if a.selftest:
        return selftest(a.fw)
    if not 1 <= len(a.fw) <= 2:
        ap.error("FILE.fw [OUTDIR]")
    a.out = a.fw[1] if len(a.fw) == 2 else None
    w, bcd, mcu, dmb = parse(open(a.fw[0], "rb").read())
    hwid = struct.unpack_from("<I", bcd, 0x20)[0] & 0xFFFF
    print("module .bcd %d B (HWID 0x%X, built %s)" % (len(bcd), hwid, bcd[8:0x18].decode().strip()))
    print(
        "MCU part %d B: %s"
        % (len(mcu), "%s, app 0x%x B at 0x80008000" % (dmb[0], len(dmb[1])) if dmb else "not a .dmb")
    )
    crc = zlib.crc32(bcd + mcu) & 0xFFFFFFFF
    print("release %d, CRC-32 0x%08x %s" % (w[5], w[4], "ok" if crc == w[4] else "WRONG (0x%08x)" % crc))
    if a.out:
        os.makedirs(a.out, exist_ok=True)
        open(os.path.join(a.out, "module.bcd"), "wb").write(bcd)
        open(os.path.join(a.out, "mcu.bin"), "wb").write(mcu)
        if dmb:
            open(os.path.join(a.out, "mcu_flash.bin"), "wb").write(flash(dmb[1]))


if __name__ == "__main__":
    main()

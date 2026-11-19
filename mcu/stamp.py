#!/usr/bin/env python3
"""Stamp the firmware's identification (src/version.c) into a built image: the commit and its date, found by the
block's magic in the .elf, .bin and .hex alike. The nix build runs it after linking (nix/default.nix), so a new
commit only re-stamps (on the host) instead of rebuilding the firmware (on a Linux builder). Then the images the tools
take, from the stamped app (NAME, at 0x80008000) and the ZD1's preloader (NAMEpre, at 0x80004000; no ident block):

  NAME.bin, NAME.hex   the ZD1's app region from 0x80004000 (DFU: `zd1 flash`, the page, zemu): the preloader, its
                       record of the app at 0x80007E00 (the app's .dmb header: what it would have written itself),
                       the app at 0x80008000
  NAME.dmb             the app as a B&W .dmb (header + app): what the module sends the MCU's bootloader, on both
                       models (docs/mcu/bootloader-hbl.md)
  NAMEapp.bin          the app alone (for bwfw.py --pack: the ZD1L's .fw)
  NAME.elf             the app's, stamped; NAMEpre.* and the .map files copied

  stamp.py --version V --built B IN_DIR OUT_DIR

`zd1 image FILE` reads the identification back.
"""

import argparse
import os
import shutil
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tools"))
from zeppelin.fw import bwfw, ihex  # noqa: E402

REGION = 0x80004000  # the ZD1's app region (B&W's DFU bootloader below it)
RECORD = 0x3E00  # the preloader's record of the app (0x80007E00), from REGION (mcu/src/mcufw.h)
APP = 0x4000  # the app, from REGION

MAGIC = b"ZD1-FW-IDENT-01\0"  # char magic[16]; char version[41]; char built[25];
FIELDS = (("version", 41), ("built", 25))


def block(version, built):
    out = b""
    for (name, size), v in zip(FIELDS, (version, built)):
        b = v.encode()
        if len(b) >= size:
            sys.exit("stamp: %s %r longer than %d bytes" % (name, v, size - 1))
        out += b + b"\0" * (size - len(b))
    return out


def find(data):
    i = data.find(MAGIC)
    if i < 0 or data.find(MAGIC, i + 1) >= 0:
        return None
    return i


def stamp_hex(text, stamp):
    lo, mem = ihex.image(text)
    i = find(bytes(mem))
    if i is None:
        sys.exit("stamp: the ident block is not in the hex (or twice)")
    start, end = lo + i + len(MAGIC), lo + i + len(MAGIC) + len(stamp)
    lines = text.splitlines()
    for n, typ, a, d in ihex.records(text):
        if typ != ihex.DATA or a + len(d) <= start or a >= end:
            continue
        d = bytearray(d)
        for k in range(len(d)):
            if start <= a + k < end:
                d[k] = stamp[a + k - start]
        lines[n] = ihex.line(a & 0xFFFF, ihex.DATA, d)
    nl = "\r\n" if "\r\n" in text else "\n"  # objcopy writes CRLF: keep the file's own line ends
    out = nl.join(lines) + nl
    if bytes(ihex.image(out)[1][i + len(MAGIC) : i + len(MAGIC) + len(stamp)]) != stamp:
        sys.exit("stamp: the stamped hex does not read back")
    return out


def stamp_binary(data, stamp):
    i = find(data)
    if i is None:
        sys.exit("stamp: the ident block is not in the image (or twice)")
    p = i + len(MAGIC)
    return data[:p] + stamp + data[p + len(stamp) :]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--version", required=True)
    ap.add_argument("--built", default="")
    ap.add_argument("src", metavar="IN_DIR")
    ap.add_argument("dst", metavar="OUT_DIR")
    a = ap.parse_args()
    src, dst = a.src, a.dst
    stamp = block(a.version, a.built)
    os.makedirs(dst, exist_ok=True)
    names = [f[: -len(".elf")] for f in os.listdir(src) if f.endswith(".elf") and not f.endswith("pre.elf")]
    for name in sorted(names):
        app = stamp_binary(open(os.path.join(src, name + ".bin"), "rb").read(), stamp)
        open(os.path.join(dst, name + ".elf"), "wb").write(
            stamp_binary(open(os.path.join(src, name + ".elf"), "rb").read(), stamp)
        )
        pre = open(os.path.join(src, name + "pre.bin"), "rb").read()
        if len(pre) > RECORD:
            sys.exit("stamp: the preloader (%d B) runs into its record at 0x%x" % (len(pre), REGION + RECORD))
        dmb = bwfw.make_dmb(app, "zd1.dmb", bytes(3), 0)
        image = pre.ljust(RECORD, b"\xff") + dmb[: bwfw.DMB_HEADER].ljust(APP - RECORD, b"\xff") + app
        assert len(image) == APP + len(app) and image[APP:] == app
        open(os.path.join(dst, name + ".bin"), "wb").write(image)
        open(os.path.join(dst, name + ".hex"), "w").write(ihex.dump(image, REGION))
        open(os.path.join(dst, name + ".dmb"), "wb").write(dmb)
        open(os.path.join(dst, name + "app.bin"), "wb").write(app)
        for f in (name + "pre.elf", name + "pre.bin", name + "pre.hex", name + ".map", name + "pre.map"):
            shutil.copyfile(os.path.join(src, f), os.path.join(dst, f))
        print("stamped %s: %s %s; preloader %d B, app %d B" % (name, a.version, a.built, len(pre), len(app)))


if __name__ == "__main__":
    main()

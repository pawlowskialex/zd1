#!/usr/bin/env python3
"""Build a BridgeCo DM870 .bcd image whose app partition runs a custom stage2 payload.

Usage:
  python3 -I mkbcd.py TEMPLATE.bcd PAYLOAD.bin OUT.bcd     # PAYLOAD linked/loaded at 0x60008000, entry = first byte
  python3 -I mkbcd.py --selftest TEMPLATE.bcd              # rebuild stock payload, must be byte-identical
  --misaligned: the payload padded to a length of 2 mod 4, which BL2 rejects (error 680): its failed-burn path

build() makes one from its parts: B&W's stage 1 (the SDRAM/clock/NAND preloader, blobs/dm870/preloader.bin), the
payload as stage 2, a model's factory CnE (section 1) and resource containers (section 2; none: an empty section, which
BL2 writes as an empty partition), the header's HWID and build date; every header and checksum is written here. The CLI takes those parts from a stock .bcd (template()); the selftest
rebuilds a stock .bcd from its own parts. Format: docs/module/boot-chain.md (".bcd images", "Boot header");
tools/zeppelin/fw/bcd.py reads it.

WARNING: the output is flashable. Only test payloads via JTAG into SDRAM first, never erase NAND
block 0 (BL1) or the BL2 partition at 0xA0000, and keep a way to reach BL2 (serial menu "Mark Image
as invalid", factory-reset strap) before flashing anything.
"""

import argparse
import os
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
from zeppelin.fw.bcd import (  # noqa: E402
    BOOT_MAGIC,
    HDR_LEN,
    MAGIC,
    PART_HDR,
    STAGE2_OFF,
    boothdr,
    crc32,
    fletcher_dm870,
    parse,
    sections,
)

# What every B&W Zeppelin image has (docs/module/boot-chain.md, ".bcd images"): the model id and software version
# (105.9112, JukeBlox 3.8.0.9112; BL2 shows them as the new image's version), the upper half of the HWID word and the
# word after it (meanings unknown), the 0xB0-byte header's version.
MODEL, SWVER, HWID_HI, WORD_24, HDR_VERSION = 0x69, 0x2398, 0x0002, 0x0007F510, 2
# section: NAND address, partition size (app, factory CnE: no address, BL2 erases the user settings; resources)
LAYOUT = ((0x240000, 0xA00000), (0, 0x20000), (0xC40000, 0x400000))
STAGE1_LOAD, STAGE2_LOAD = 0x0040DA00, 0x8000  # stage 1 into SRAM; stage 2 at 0x60000000 + this
STAGE1_OFF = 0x40  # the boot header, then stage 1's code; stage 1 (its CRC) runs from the branch at 0x18
STAGE1_PAD = b" "  # B&W pad stage 1 to STAGE2_OFF with spaces


def boot_header(stage1, stage2, nand):
    """The app section's start: the boot header (stage 1's parameters inside its CRC: the branch over them, stage 2's
    load address, NAND address, length and checksum), stage 1, its padding to STAGE2_OFF."""
    n = STAGE1_OFF - 0x18 + len(stage1)
    if n % 8 or STAGE1_OFF + len(stage1) > STAGE2_OFF:
        raise ValueError("stage 1 is %d B: it fits in 0x%x and its region is whole 8-byte words" % (len(stage1), n))
    s1 = (
        struct.pack(
            "<10I",
            0xEA000008,
            0xA5A5A5A5,
            STAGE2_LOAD,
            nand + PART_HDR + STAGE2_OFF,
            len(stage2),
            fletcher_dm870(stage2),
            0,
            0,
            0,
            0,
        )
        + stage1
    )
    head = struct.pack("<4s5I", BOOT_MAGIC, 1, 0xA5A5A5A5, STAGE1_LOAD, n // 8, crc32(s1))
    return (head + s1).ljust(STAGE2_OFF, STAGE1_PAD)


def build(
    stage1, payload, cne, resources, hwid, date, model=MODEL, swver=SWVER, hwid_hi=HWID_HI, word_24=WORD_24, pad=4
):
    """A .bcd: `payload` as stage 2 (loaded and entered at 0x60008000) behind `stage1`, the factory CnE `cne`, the
    resource containers `resources` (a list, up to 8: the header keeps their sizes); `date` the build's, 14 digits."""
    payload += b"\0" * (-len(payload) % pad)  # BL2's web updater rejects a section whose length is not a multiple of 4
    app = boot_header(stage1, payload, LAYOUT[0][0]) + payload
    datas = [app, cne, b"".join(resources)]
    for (_, psize), d in zip(LAYOUT, datas):
        if len(d) > psize - PART_HDR:
            raise ValueError("a section of %d B doesn't fit its partition (0x%x)" % (len(d), psize))
    if len(date) != 14 or not date.isdigit() or len(resources) > 8:
        raise ValueError("a date is YYYYMMDDhhmmss; up to 8 resource containers")
    hdr = bytearray(HDR_LEN)
    hdr[0:0x18] = MAGIC + struct.pack("<I", 1) + date.encode().ljust(16)
    struct.pack_into("<5I", hdr, 0x18, model, swver, hwid_hi << 16 | hwid, word_24, HDR_VERSION)
    pos = HDR_LEN + 8
    for i, ((nand, psize), d) in enumerate(zip(LAYOUT, datas)):
        struct.pack_into("<5I", hdr, 0x30 + 0x20 * i, pos, nand, len(d), crc32(d), psize)
        pos += len(d)
    struct.pack_into("<8I", hdr, 0x90, *[len(r) for r in resources] + [0] * (8 - len(resources)))
    struct.pack_into("<I", hdr, 0x2C, crc32(hdr))
    out = bytes(hdr) + b"DMP 3.x\0" + b"".join(datas)
    return out + b"\xff" * (-len(out) % 0x400)  # B&W's files end 0xFF-padded to 1 KiB


def template(t):
    """A stock .bcd's parts, as build()'s keyword arguments (all but the payload)."""
    h, _ = parse(t)
    app, cne, res = sections(t)
    b = boothdr(app["data"])
    sizes = [x for x in struct.unpack_from("<8I", t, 0x90) if x]
    if b is None or sum(sizes) != res["size"]:
        raise ValueError("the template has no boot header, or its resource containers don't add up")
    resources, o = [], 0
    for x in sizes:
        resources.append(res["data"][o : o + x])
        o += x
    return dict(
        stage1=app["data"][STAGE1_OFF : 0x18 + b["stage1_words"] * 8],
        cne=cne["data"],
        resources=resources,
        hwid=h["hwid"] & 0xFFFF,
        date=h["build"],
        model=h["verA"],
        swver=h["verB"],
        hwid_hi=h["hwid"] >> 16,
        word_24=h["u24"],
    )


def stage2(t):
    """A stock .bcd's stage 2."""
    return sections(t)[0]["data"][STAGE2_OFF:]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("template", metavar="TEMPLATE.bcd")
    ap.add_argument("payload", nargs="?", metavar="PAYLOAD.bin")
    ap.add_argument("out", nargs="?", metavar="OUT.bcd")
    ap.add_argument("--selftest", action="store_true", help="rebuild the template's own payload: byte-identical?")
    ap.add_argument("--misaligned", action="store_true", help="a section length BL2 rejects (2 mod 4)")
    a = ap.parse_args()
    t = open(a.template, "rb").read()
    parts = template(t)
    if a.selftest:
        s2 = stage2(t)
        ok = build(payload=s2, **parts) == t
        print("stage2 checksum 0x%08x" % fletcher_dm870(s2))
        print("rebuilt from its parts byte-identical:", ok)
        return 0 if ok else 1
    if not a.out:
        ap.error("TEMPLATE.bcd PAYLOAD.bin OUT.bcd")
    p = open(a.payload, "rb").read()
    if a.misaligned:
        p += b"\0" * ((2 - len(p)) % 4)  # length = 2 mod 4
        out = build(payload=p, pad=2, **parts)
    else:
        out = build(payload=p, **parts)
    open(a.out, "wb").write(out)
    print("wrote %s (%d bytes, payload %d bytes)" % (a.out, len(out), len(p)))
    return 0


if __name__ == "__main__":
    sys.exit(main())

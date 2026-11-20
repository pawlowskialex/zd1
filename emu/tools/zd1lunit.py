#!/usr/bin/env python3
"""A stock Zeppelin Air with Lightning (ZD1L) for the emulator, from the test unit and a B&W ZD1L .fw: the NAND with
the ZD1L's HWID (0xBE), its BL2 (blobs/zd1l/module/bl2.bin) and the .fw's module image installed as BL2 writes it,
and the MCU flash with the ZD1L's bootloader ("Net HBL") and the .fw's app at 0x80008000. Emulation only.

usage: zd1lunit.py OUTDIR [FILE.fw]        (default: blobs/zd1l/zd1lv20435.fw, B&W's 2.04.35)

Then (docs/mcu/bootloader-hbl.md): the key-hold into its updater, an upload to BL2's web updater, both chips:
    zemu run OUTDIR/nand.bin --nand --mcu OUTDIR/mcu_flash.bin --fwd 18080:80 \\
      --renode 'zd1:flashc BootprotSize 0x8000' --renode 'zd1:board Adc "2:0x300"' \\
      --renode 'zd1:board Pins "0:PA08=0,1500:PA08=1"' --wait mcu 'BSL Ready' --sleep 30 \\
      --host 'python3 -I tools/zeppelin/knos/bl2web.py 127.0.0.1:18080 zd1l.fw --yes' ...
"""

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..")
sys.path.insert(0, os.path.join(ROOT, "tools"))
from zeppelin.fw import bcd, bwfw  # noqa: E402

BL2 = (0xA0000, 0x240000)  # BL2's region (docs/module/boot-chain.md)
PERSPARAM = (0x20000, 0x40000)


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    out = sys.argv[1]
    fw = sys.argv[2] if len(sys.argv) == 3 else os.path.join(ROOT, "blobs", "zd1l", "zd1lv20435.fw")
    _, module, _, dmb = bwfw.parse(open(fw, "rb").read())
    if not dmb or not dmb[0].startswith("ZD1L"):
        sys.exit("%s: not a ZD1L .fw (its MCU part is no ZD1L .dmb)" % fw)
    nand = bytearray(open(os.path.join(ROOT, "emu", "img", "test-unit", "nand.bin"), "rb").read())
    # PersParam: the HWID is the 4 bytes after the reversed key "DIWH"
    at = nand.find(b"DIWH", *PERSPARAM)
    if at < 0:
        sys.exit("no HWID in the test unit's PersParam")
    nand[at + 4 : at + 8] = bytes([0xBE, 0, 0, 0])
    bl2 = open(os.path.join(ROOT, "blobs", "zd1l", "module", "bl2.bin"), "rb").read()
    if len(bl2) != BL2[1] - BL2[0]:
        sys.exit("bl2.bin is not BL2's whole region")
    nand[BL2[0] : BL2[1]] = bl2
    bcd.install(nand, module, log=lambda s: print("  " + s))
    os.makedirs(out, exist_ok=True)
    open(os.path.join(out, "nand.bin"), "wb").write(nand)
    open(os.path.join(out, "mcu_flash.bin"), "wb").write(bwfw.flash(dmb[1], zd1l=True))
    print(
        "%s: nand.bin (HWID 0xBE, the ZD1L's BL2, %s's module image), mcu_flash.bin (the HBL, %s)"
        % (out, os.path.basename(fw), dmb[0])
    )


if __name__ == "__main__":
    main()

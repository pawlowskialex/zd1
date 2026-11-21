"""A stock MCU app as a raw image from 0x80000000 (re/gen/mcu-app/image.bin, the A7's re/gen/mcu-app-a7/image.bin;
re/README.md, "The MCU image"): big-endian flash words, C strings, and the .data load image that RAM 0x8-END starts
from (.bss after it is zero at boot, so it has no bytes here). Where .data loads from and ends is read from the app's
start-up code (its literal pool at 0x8000807C: LMA, 8, END): ZD1 2.04.05 0x80034AD0 / 0x3630, A7 2.04.77
0x80033A88 / 0x3068."""

import struct

FLASH = 0x80000000
DATA_LMA, DATA_VMA, DATA_END = 0x80034AD0, 0x8, 0x3630  # the ZD1's: in_data() without an image
CRT0_POOL = 0x807C  # the start-up code's [.data LMA, VMA, END]


def in_flash(a):
    return FLASH <= a < FLASH + 0x40000


def in_data(a, end=DATA_END):
    return DATA_VMA <= a < end


def data_layout(b):
    """(LMA, END) of .data from the start-up code's literal pool; the ZD1's if it isn't there."""
    if len(b) >= CRT0_POOL + 12:
        lma, vma, end = struct.unpack_from(">3I", b, CRT0_POOL)
        if vma == DATA_VMA and in_flash(lma) and DATA_VMA < end < 0x10000:
            return lma, end
    return DATA_LMA, DATA_END


class Image:
    def __init__(self, path):
        self.b = open(path, "rb").read()
        self.data_lma, self.data_end = data_layout(self.b)

    def in_data(self, a):
        return in_data(a, self.data_end)

    def offset(self, a):
        """File offset of a flash address, or of an initialised-RAM address in the .data load image; else None."""
        if in_flash(a) and a - FLASH < len(self.b):
            return a - FLASH
        if self.in_data(a):
            return self.data_lma - FLASH + a - DATA_VMA
        return None

    def bytes(self, a, n):
        o = self.offset(a)
        return self.b[o : o + n] if o is not None else b""

    def word(self, a):
        w = self.bytes(a, 4)
        return struct.unpack(">I", w)[0] if len(w) == 4 else None

    def cstr(self, a, least=4, most=100, chars=(9, 10)):
        """The NUL-terminated printable string at a (least <= length < most), or None."""
        o = self.offset(a)
        if o is None:
            return None
        e = self.b.find(b"\0", o)
        s = self.b[o : e if e >= 0 else len(self.b)]
        if least <= len(s) < most and all(32 <= c < 127 or c in chars for c in s):
            return s.decode()
        return None

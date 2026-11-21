#!/usr/bin/env python3
"""Vanilla Linux 2.6.24.7 made ready to build modules for Nokia's kernel (nix/module/wifi.nix runs it on the unpacked
tree; README.md here has why):

    kernel.py LINUX_DIR KERNEL_BIN

- Nokia's own configuration, from the image (the gzip between IKCFG_ST and IKCFG_ED), as LINUX_DIR/.config.
- A stand-in ARCH_SULU, only what generic headers need: an ARM926T, SDRAM at 0x60000000 (PHYS_OFFSET), Versatile's
  other machine headers. With it, `oldconfig` keeps Nokia's configuration but for their Sulu drivers.
- The two Makefile rules make >= 3.82 rejects, split.
- clang: the GCC-only ARM flags made optional, `asm-offsets.s` without the integrated assembler (it is assembly text
  with "->" markers), `current_thread_info()` reading sp with inline asm (clang has no local register variables
  bound to sp), and -fno-delete-null-pointer-checks -fwrapv, as later kernels pass: ARM's BUG() here is a store
  through NULL (no CONFIG_DEBUG_BUGVERBOSE), which clang would delete.
"""

import glob
import gzip
import os
import shutil
import sys


def nokia_config(kernel_bin):
    d = open(kernel_bin, "rb").read()
    s = d.index(b"IKCFG_ST") + len(b"IKCFG_ST")
    return gzip.decompress(d[s : d.index(b"IKCFG_ED", s)]).decode()


def main(k, kernel_bin):
    def sub(path, old, new):
        p = os.path.join(k, path)
        s = open(p).read()
        if s.count(old) != 1:
            sys.exit(f"{path}: not exactly one {old!r} (another kernel version?)")
        open(p, "w").write(s.replace(old, new))

    open(os.path.join(k, ".config"), "w").write(nokia_config(kernel_bin))

    # make >= 3.82: no explicit and pattern targets in one rule
    rule = "\t$(Q)mkdir -p include/linux include/config\n\t$(Q)$(MAKE) $(build)=scripts/kconfig $@\n"
    sub(
        "Makefile",
        "config %config: scripts_basic outputmakefile FORCE\n" + rule,
        "config: scripts_basic outputmakefile FORCE\n" + rule + "%config: scripts_basic outputmakefile FORCE\n" + rule,
    )
    mod = "\t$(cmd_crmodverdir)\n\t$(Q)$(MAKE) KBUILD_MODULES=$(if $(CONFIG_MODULES),1) \\\n\t$(build)=$(build-dir)\n"
    sub(
        "Makefile",
        "/ %/: prepare scripts FORCE\n" + mod,
        "/: prepare scripts FORCE\n" + mod + "%/: prepare scripts FORCE\n" + mod,
    )

    # the stand-in machine
    sub(
        "arch/arm/Kconfig",
        "config ARCH_VERSATILE\n",
        'config ARCH_SULU\n\tbool "BridgeCo Sulu (stand-in for Nokia\'s)"\n\tselect GENERIC_TIME\n'
        "\tselect GENERIC_CLOCKEVENTS\n\tselect CPU_ARM926T\n\tselect FIQ\n\nconfig ARCH_VERSATILE\n",
    )
    sub(
        "arch/arm/mm/Kconfig",
        'config CPU_ARM926T\n\tbool "Support ARM926T processor"\n\tdepends on ARCH_INTEGRATOR',
        'config CPU_ARM926T\n\tbool "Support ARM926T processor"\n\tdepends on ARCH_SULU || ARCH_INTEGRATOR',
    )
    sub(
        "arch/arm/Makefile",
        " machine-$(CONFIG_ARCH_VERSATILE)",
        " machine-$(CONFIG_ARCH_SULU)\t   := sulu\n machine-$(CONFIG_ARCH_VERSATILE)",
    )
    os.makedirs(os.path.join(k, "arch/arm/mach-sulu"), exist_ok=True)
    open(os.path.join(k, "arch/arm/mach-sulu/Makefile"), "w").close()
    arch = os.path.join(k, "include/asm-arm/arch-sulu")
    os.makedirs(arch, exist_ok=True)
    for h in glob.glob(os.path.join(k, "include/asm-arm/arch-versatile/*.h")):
        shutil.copy(h, arch)
    phys = "#define PHYS_OFFSET\tUL(0x%08x)"
    sub("include/asm-arm/arch-sulu/memory.h", phys % 0, phys % 0x60000000)

    # clang
    sub(
        "arch/arm/Makefile",
        "KBUILD_CFLAGS\t+=-fno-omit-frame-pointer -mapcs -mno-sched-prolog",
        "KBUILD_CFLAGS\t+=-fno-omit-frame-pointer $(call cc-option,-mapcs,) $(call cc-option,-mno-sched-prolog,)",
    )
    sub(
        "arch/arm/Makefile",
        "CFLAGS_ABI\t:=-mabi=aapcs-linux -mno-thumb-interwork",
        "CFLAGS_ABI\t:=-mabi=aapcs-linux $(call cc-option,-mno-thumb-interwork,)",
    )
    sub(
        "Kbuild",
        "# Default sed regexp - multiline due to syntax constraints\n",
        "CFLAGS_asm-offsets.o += $(call cc-option,-no-integrated-as,)\n\n"
        "# Default sed regexp - multiline due to syntax constraints\n",
    )
    sub(
        "include/asm-arm/thread_info.h",
        '\tregister unsigned long sp asm ("sp");\n',
        '\tunsigned long sp;\n\tasm ("mov %0, sp" : "=r" (sp));\n',
    )
    sub(
        "Makefile",
        "\t\t   -fno-strict-aliasing -fno-common \\\n",
        "\t\t   -fno-strict-aliasing -fno-common \\\n"
        "\t\t   $(call cc-option,-fno-delete-null-pointer-checks,) $(call cc-option,-fwrapv,) \\\n",
    )


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(*sys.argv[1:])

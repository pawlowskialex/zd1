#!/usr/bin/env python3
"""The module OS images (our userspace on Nokia's 2.6.24 kernel) from module/os/os.toml and the compiled parts. Nothing
here touches hardware.

    nix build -o emu/img/os .#os-image          the images (nix/os-image.nix runs `assemble.py assemble` on
                                                `nix build .#module-os-parts` and the Nokia kernel and modules in
                                                blobs/); `zemu run` builds them when it needs them
    python3 module/os/assemble.py mini TAG      the minimal test image printing TAG: emu/img/mini-TAG.bin

Nix builds the files git knows: `git add` a new source file first. No image carries a Wi-Fi network: the speaker's
settings do (`zd1 wifi`, docs/module/settings.md).

The images (emu/img/os/; docs/module/storage.md):
    system.cpio.gz          a system slot's root (busybox, the full zd1, /etc, Nokia's modules and firmware, ALSA,
                            Wi-Fi): what `zd1 update` installs (deterministic: sorted, root-owned, mtime 0)
    system-debug.cpio.gz    the same with zd1's debug build (os.toml [image.debug])
    initramfs-full.cpio.gz  the same system (release) as an initramfs, its init as /init: the app partition's copy
    payload.bin, os.bcd     the flashable app image (B&W's preloader + our stub, the kernel and that initramfs; the
                            relay, BL2): the stub asks the MCU for a slot; with no slot that gets ready the system runs
                            from RAM, and with the slots empty (a first install) copies itself into one
                            (zd1-slot install-self)
    os-zd1l.bcd             the same for the Zeppelin Air with Lightning (its HWID and CnE, no resources): the module
                            part of its B&W .fw for BL2's web updater (`nix build .#zd1l-fw`)
    payload_debug.bin, os-debug.bcd   os.bcd with the debug services on by default (os.toml [image.debug])
    nand_os.bin             the emulator's test unit with os.bcd installed, slots empty (`zemu run os --nand --mcu`:
                            slot A and B fail, the system from RAM fills slot A)
    payload_dev.bin         emulator dev boot (`zemu run os`): the debug system as one plain initramfs at
                            0x61000000, zd1.selftest zd1.debug
"""

import argparse
import gzip
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(REPO, "tools"))
from zeppelin.fw import bcd, mkbcd  # noqa: E402

IMG = os.path.join(REPO, "emu", "img")
PY = [sys.executable, "-I"]


def log(msg):
    print(msg, flush=True)


def sh(cmd, **kw):
    log("  $ " + " ".join(cmd))
    return subprocess.run(cmd, cwd=REPO, check=True, **kw)


def copy_tree(src, dst):
    """copytree out of the nix store, made writable (the store is read-only)."""
    shutil.copytree(src, dst, symlinks=True, dirs_exist_ok=True)
    for dp, dns, fs in os.walk(dst):
        for n in dns + fs:
            q = os.path.join(dp, n)
            if not os.path.islink(q):
                os.chmod(q, os.stat(q).st_mode | 0o200)
    os.chmod(dst, os.stat(dst).st_mode | 0o200)


# ---- assembly ----------------------------------------------------------------------------------------------------
def os_release(version, built):
    """The system's identification, /usr/lib/os-release (init adds the variant: the image's command line decides)."""
    return 'NAME="zd1 module OS"\nID=zd1\nVERSION_ID="%s"\nBUILD_DATE="%s"\n' % (version, built)


def content_id(path):
    """12 hex digits of what Nix hashes a path by (its NAR's SHA-256: every name, kind, mode and content): an
    identity that changes only when what is in it does."""
    out = subprocess.run(["nix-hash", "--type", "sha256", path], check=True, capture_output=True, text=True).stdout
    return out.strip()[:12]


def link_programs(root, target, names):
    """A link to the multicall per program in root/bin, `target` relative to it (busybox's /sbin is a link to /bin:
    /sbin/init, the kernel's choice, is /bin/init). Relative, so a root mounted elsewhere resolves into itself:
    zd1-slot checks a new slot's /sbin/init that way."""
    for b in names:
        os.symlink(target, os.path.join(root, "bin", b))


def make_system(cfg, parts, root, version, built, mcu_app, debug=False):
    """A system slot's tree, the whole root the kernel mounts (read-only): busybox, the full zd1 (/usr/bin/zd1: the
    core programs and the rest, a link each; /sbin/init), /etc, Nokia's kernel modules and Wi-Fi firmware,
    wpa_supplicant, ALSA's configuration and tools, the MCU's app of the same release (/usr/lib/zd1/mcu/zd1.dmb: the
    MCU follows its OS, docs/protocol/net.md); mount points for what init mounts (/dev, /run, /tmp, /var: tmpfs;
    /data its partition). `debug`: zd1's debug build, with its programs' links."""
    v, a = cfg["vendor"], cfg["apps"]
    vroot = os.path.join(REPO, v["root"])
    copy_tree(os.path.join(parts, "busybox"), root)
    for d in (
        "dev",
        "proc",
        "sys",
        "tmp",
        "var",
        "mnt",
        "root",
        "run",
        "data",
        "etc",
        "lib",
        "usr/bin",
        "usr/sbin",
        "usr/share/zd1",
        "usr/lib",
    ):
        os.makedirs(os.path.join(root, d), exist_ok=True)
    usr = os.path.join(root, "usr")
    for d in ("modules", "firmware"):
        os.symlink("/usr/lib/" + d, os.path.join(root, "lib", d))
    kver = "2.6.24.7"
    for m in v["modules"]:
        dst = os.path.join(usr, "lib/modules", kver, "kernel", m + ".ko")
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copyfile(os.path.join(vroot, "lib/modules", kver, "kernel", m + ".ko"), dst)
    for f in v["firmware"]:  # lib/firmware/... -> usr/lib/firmware/...
        src, dst = os.path.join(vroot, f), os.path.join(usr, f)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        if os.path.islink(src):
            os.symlink(os.readlink(src), dst)
        else:
            shutil.copyfile(src, dst)
    copy_tree(os.path.join(parts, "alsa-lib", "share", "alsa"), os.path.join(usr, "share/alsa"))  # alsa.conf
    copy_tree(os.path.join(parts, "alsa-utils", "bin"), os.path.join(usr, "bin"))  # aplay, ...
    copy_tree(os.path.join(parts, "wpa_supplicant", "bin"), os.path.join(usr, "sbin"))  # wpa_supplicant, wpa_cli
    mc = os.path.join(usr, "bin", a["multicall"])
    shutil.copyfile(os.path.join(parts, "apps-debug" if debug else "apps", "bin", a["multicall"]), mc)
    os.chmod(mc, 0o755)
    link_programs(root, "../usr/bin/" + a["multicall"], a["core"])
    link_programs(usr, a["multicall"], a["bins"] + (a["debug"] if debug else []))
    shutil.copytree(os.path.join(REPO, cfg["image"]["overlay"]), root, symlinks=True, dirs_exist_ok=True)
    open(os.path.join(root, "etc/modules"), "w").write("\n".join(v["load"]) + "\n")
    open(os.path.join(usr, "lib/os-release"), "w").write(os_release(version, built))
    if mcu_app:
        os.makedirs(os.path.join(usr, "lib/zd1/mcu"), exist_ok=True)
        shutil.copyfile(mcu_app, os.path.join(usr, "lib/zd1/mcu/zd1.dmb"))
    for f in ("os-release", "resolv.conf"):  # written at run time: / is read-only
        os.symlink("/run/" + f, os.path.join(root, "etc", f))


def cpio(root, out, name, slot=False):
    """`root` as out/NAME.cpio and out/NAME.cpio.gz (slot: a system slot's root, /dev/console and /dev/null its only
    device nodes)."""
    c = os.path.join(out, name + ".cpio")
    sh(PY + [os.path.join(HERE, "mkcpio.py")] + (["--slot"] if slot else []) + [root, c])
    raw = open(c, "rb").read()
    open(c + ".gz", "wb").write(gzip.compress(raw, 9, mtime=0))
    log("  %s %d B (%d B gzip)" % (name, len(raw), os.path.getsize(c + ".gz")))


def payload(cfg, parts, initrd, out, cmdline_extra="", how="slots"):
    """The stub + kernel + initramfs as one app payload (mkpayload.py). `how`: "slots", the stub asking the MCU what to
    start, a system slot or the initramfs (--slots); "dev", the emulator's direct boot, the initramfs started as it is,
    a plain one loaded lower.
    The kernel goes in deflated (half the size): the stub inflates it. The command line ends in `zd1.boot=ID`, the boot
    image's identity: the content_id of the stub, the kernel and how they start (the arguments, the command line before
    it), so a commit that changes none of them builds the same one. The OS reports it (the State record's `boot`),
    `zd1 image` reads it from a .bcd, and the page rewrites the app partition only when they differ. The initramfs
    isn't in it: it is the release's system (the rescue), new in every release, and a rescue older than the slots is
    still the way back (it runs, plays and takes updates); it is renewed whenever the stub or the kernel are.
    Then `zd1.image=ID`, the whole image's identity (the same with the initramfs): the stub tells it to our MCU, which
    keeps the last one and starts a new image's rescue to install itself into a slot (mcu/src/modboot.c)."""
    k = cfg["kernel"]
    args = {"slots": ["--slots"], "dev": ["--initrd-at", "0x61000000"]}[how] + ["--deflate-kernel"]
    files = [os.path.join(parts, "boot", "linuxboot.bin"), os.path.join(REPO, k["image"]), initrd]
    cmdline = (k["cmdline"] + " " + cmdline_extra).strip()
    with tempfile.TemporaryDirectory() as d:
        for n, p in zip(("stub", "kernel"), files):
            shutil.copyfile(p, os.path.join(d, n))
        open(os.path.join(d, "how"), "w").write(" ".join(args) + "\n" + cmdline + "\n")
        boot = content_id(d)
        shutil.copyfile(initrd, os.path.join(d, "initramfs"))
        cmdline += " zd1.boot=" + boot + " zd1.image=" + content_id(d)
    sh(PY + [os.path.join(HERE, "mkpayload.py")] + files + [out] + args + ["--cmdline", cmdline])


# The models' .bcd parts besides the payload (blobs/README.md): HWID and factory CnE. B&W's preloader is common; the
# resources (the stock app's web UI, which our OS doesn't read) are left empty: BL2 writes the partition empty.
MODELS = {"zd1": 0xBD, "zd1l": 0xBE}


def bcd_image(payload_bin, out, model, built):
    """The payload as a model's .bcd for BL2 (its web updater, the relay's XModem): B&W's preloader, the model's HWID
    and factory CnE, no resources, our build date (docs/module/boot-chain.md, ".bcd images")."""
    blob = lambda name: open(os.path.join(REPO, "blobs", name), "rb").read()
    image = mkbcd.build(
        stage1=blob("dm870/preloader.bin"),
        payload=open(payload_bin, "rb").read(),
        cne=blob("%s/module/cne.bin" % model),
        resources=[],
        hwid=MODELS[model],
        date="".join(c for c in built if c.isdigit()).ljust(14, "0"),
    )
    open(out, "wb").write(image)
    log("  %s %d B" % (os.path.basename(out), len(image)))


def install(base_nand, image, out):
    """The test unit's NAND with `image` installed the way BL2 leaves it."""
    nand = bytearray(open(base_nand, "rb").read())
    bcd.install(nand, open(image, "rb").read())
    open(out, "wb").write(nand)


def assemble(cfg, parts, out, version, built, base_nand, mcu_app):
    """Every image of os.toml into `out` (the nix build runs this)."""
    os.makedirs(out, exist_ok=True)
    log("system")
    sysroot = os.path.join(out, ".system")
    make_system(cfg, parts, sysroot, version, built, mcu_app)
    cpio(sysroot, out, "system", slot=True)
    # the same system as the app partition's initramfs (gzip'd: it goes to the stub's default destination), its init
    # as /init: what runs when no slot does, on both models
    log("initramfs-full")
    os.symlink("sbin/init", os.path.join(sysroot, "init"))
    cpio(sysroot, out, "initramfs-full")
    shutil.rmtree(sysroot)
    img = cfg["image"]
    variants = [("", "")]
    if img.get("debug"):  # the debug image: its debug services on by default
        variants.append(("-debug", img["debug"]["cmdline"]))
    for suffix, cmdline in variants:
        p = os.path.join(out, "payload%s.bin" % suffix.replace("-", "_"))
        payload(cfg, parts, os.path.join(out, "initramfs-full.cpio.gz"), p, cmdline)
        bcd_image(p, os.path.join(out, "os%s.bcd" % suffix), "zd1", built)
    bcd_image(os.path.join(out, "payload.bin"), os.path.join(out, "os-zd1l.bcd"), "zd1l", built)
    install(base_nand, os.path.join(out, "os.bcd"), os.path.join(out, "nand_os.bin"))
    log("system-debug")
    make_system(cfg, parts, sysroot, version, built, mcu_app, debug=True)
    cpio(sysroot, out, "system-debug", slot=True)
    # the emulator's dev boot: the debug system as one plain initramfs loaded straight into SDRAM, its init as /init
    log("initramfs-dev")
    os.symlink("sbin/init", os.path.join(sysroot, "init"))
    cpio(sysroot, out, "initramfs-dev")
    shutil.rmtree(sysroot)
    payload(
        cfg,
        parts,
        os.path.join(out, "initramfs-dev.cpio"),
        os.path.join(out, "payload_dev.bin"),
        "zd1.selftest zd1.debug",
        how="dev",
    )


def mini(tag):
    """The mini image printing TAG at boot (tells two flashed builds apart): emu/img/mini-TAG.bin."""
    expr = (
        "let e = builtins.getEnv; in "
        '(builtins.getFlake (e "ZD1_FLAKE")).packages.${builtins.currentSystem}.boot.override { tag = e "ZD1_TAG"; }'
    )
    res = subprocess.run(
        ["nix", "build", "--impure", "--no-link", "--print-out-paths", "--expr", expr],
        cwd=REPO,
        check=True,
        capture_output=True,
        text=True,
        env=dict(os.environ, ZD1_FLAKE="git+file://" + REPO, ZD1_TAG=tag),
    ).stdout.strip()
    dst = os.path.join(IMG, "mini-%s.bin" % tag)
    os.makedirs(IMG, exist_ok=True)
    if os.path.exists(dst):
        os.remove(dst)
    shutil.copyfile(os.path.join(res, "mini-%s.bin" % tag), dst)
    return dst


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", nargs="+", help="mini TAG | assemble (the nix build)")
    ap.add_argument("--parts", help="assemble: the compiled parts (module-os-parts)")
    ap.add_argument("--base-nand", help="assemble: the NAND to install the images into (the test unit)")
    ap.add_argument("--version", help="assemble: the version in /etc/os-release")
    ap.add_argument("--built", default="", help="assemble: the build date in /etc/os-release")
    ap.add_argument("--out", help="assemble: where the images go")
    ap.add_argument("--mcu-app", help="assemble: the MCU's app of the same release (zd1.dmb), carried by the system")
    a = ap.parse_args()
    cfg = tomllib.load(open(os.path.join(HERE, "os.toml"), "rb"))
    if a.cmd[:1] == ["assemble"]:
        assemble(cfg, a.parts, a.out, a.version, a.built, a.base_nand, a.mcu_app)
    elif a.cmd[:1] == ["mini"] and len(a.cmd) == 2:
        log(os.path.relpath(mini(a.cmd[1]), REPO))
    else:
        ap.error("unknown command: %s" % " ".join(a.cmd))


if __name__ == "__main__":
    main()

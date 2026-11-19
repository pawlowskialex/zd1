# The module OS images (module/os/assemble.py assemble, from os.toml): the system image, the flashable os.bcd,
# os-debug.bcd and os-zd1l.bcd (the system in the app partition too), the emulator's dev payload, and the test unit's
# NAND with os.bcd installed.
{
  stdenvNoCC,
  python3,
  nix,
  src,
  parts,
  test-unit,
  version,
  built,
}:
stdenvNoCC.mkDerivation {
  name = "zd1-os-image";
  src = src [
    "module/os/assemble.py"
    "module/os/os.toml"
    "module/os/mkcpio.py"
    "module/os/rootfs"
    "module/os/mkpayload.py"
    "tools/zeppelin/__init__.py"
    "tools/zeppelin/fw/bcd.py"
    "tools/zeppelin/fw/mkbcd.py"
    "blobs/dm870/preloader.bin"
    "blobs/zd1/module/cne.bin"
    "blobs/zd1l/module/cne.bin"
    "blobs/nokia-hd1"
  ];
  nativeBuildInputs = [
    python3
    nix # nix-hash: the boot image's identity by content (assemble.py, content_id)
  ];
  buildPhase = ''
    python3 -I module/os/assemble.py assemble --parts ${parts} --base-nand ${test-unit}/nand.bin \
      --version ${version} --built "${built}" --out $out
  '';
  dontInstall = true;
  dontFixup = true;
}

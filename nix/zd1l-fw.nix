# Our firmware for the Zeppelin Air with Lightning (ZD1L) as one B&W over-the-air .fw: both chips from one file through
# the ZD1L's BL2 web updater (docs/mcu/stock-firmware.md). The module part is os-zd1l.bcd (the stub, the kernel and the
# whole system as the rescue, which installs itself into a slot at the first start; the ZD1L's HWID and CnE, no
# resources); the MCU part our app (at 0x80008000, as on
# every model: `mcu`'s zd1app.bin), under the name and version of B&W's last ZD1L release, which its BL2 took on a unit.
{
  stdenvNoCC,
  python3,
  src,
  os-image,
  mcu,
}:
stdenvNoCC.mkDerivation {
  name = "zd1-zd1l-fw";
  src = src [
    "tools/zeppelin/__init__.py"
    "tools/zeppelin/paths.py"
    "tools/zeppelin/fw/bcd.py"
    "tools/zeppelin/fw/bwfw.py"
    "tools/zeppelin/xmodem.py"
  ];
  nativeBuildInputs = [ python3 ];
  buildPhase = ''
    mkdir -p $out
    python3 -I tools/zeppelin/fw/bwfw.py --pack $out/zd1l.fw ${os-image}/os-zd1l.bcd ${mcu}/zd1app.bin \
      --name ZD1L_v2-04-35.dmb --version 2.04.35
    python3 -I tools/zeppelin/fw/bwfw.py $out/zd1l.fw
  '';
  dontInstall = true;
  dontFixup = true;
}

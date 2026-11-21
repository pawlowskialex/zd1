# Our firmware for a model whose BL2 takes a B&W over-the-air .fw (the Zeppelin Air with Lightning, the A7): both chips
# from one file through that BL2's web updater (docs/mcu/stock-firmware.md, docs/models/). The module part is
# os-MODEL.bcd (the stub, the kernel and the whole system as the rescue, which installs itself into a slot at the first
# start; the model's HWID and CnE, no resources); the MCU part our app (at 0x80008000, as on every model: `mcu`'s
# zd1app.bin, the same for every model: it tells them apart itself, mcu/src/model.c), under the name and version of
# B&W's last release for the model, which its BL2 takes.
{
  stdenvNoCC,
  python3,
  src,
  os-image,
  mcu,
  # zd1l | a7: os-MODEL.bcd, MODEL.fw
  model,
  # B&W's last release's .dmb name and version
  dmb,
  release,
}:
stdenvNoCC.mkDerivation {
  name = "zd1-${model}-fw";
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
    python3 -I tools/zeppelin/fw/bwfw.py --pack $out/${model}.fw ${os-image}/os-${model}.bcd ${mcu}/zd1app.bin \
      --name ${dmb} --version ${release}
    python3 -I tools/zeppelin/fw/bwfw.py $out/${model}.fw
  '';
  dontInstall = true;
  dontFixup = true;
}

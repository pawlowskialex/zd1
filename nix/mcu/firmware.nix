# The MCU firmware (mcu/Makefile): the app, $out/NAME.{elf,hex,bin,map} (at 0x80008000), and the ZD1's preloader,
# $out/NAMEpre.{elf,hex,bin,map} (at 0x80004000); the stamp step (mcu/stamp.py) makes the images the tools take.
#   emulator     the emulator's build (packages mcu-emu): the console on USART0 as well, where zemu reads and types
#                it (no USB host there; ZD1_EMU)
{
  lib,
  stdenv,
  avr32-toolchain,
  src,
  name ? "zd1",
  emulator ? false,
}:
stdenv.mkDerivation {
  inherit name;
  src = src [
    "mcu/Makefile"
    "mcu/src"
    "mcu/preloader"
    "mcu/config"
    "mcu/dfp-include"
    "mcu/asf"
    "mcu/link_zd1.lds"
  ];
  sourceRoot = "source/mcu";
  nativeBuildInputs = [ avr32-toolchain ];
  makeFlags = [
    "OUT=${name}"
  ]
  ++ lib.optional emulator "EXTRA_CFLAGS=-DZD1_EMU";
  enableParallelBuilding = true;
  installPhase = ''
    mkdir -p $out
    cp build/${name}{,pre}.{elf,hex,bin,map} $out/
  '';
  dontFixup = true;
  meta.platforms = [ "x86_64-linux" ];
}

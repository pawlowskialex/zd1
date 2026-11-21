# zd1-app's icon (host/app/icon/zd1.svg) in the formats the systems want: PNGs at the usual sizes, macOS's .icns and
# Windows' .ico (each holding its sizes), and the SVG itself (Linux's hicolor theme takes it as it is).
{
  runCommand,
  librsvg,
  libicns,
  icoutils,
}:
runCommand "zd1-app-icons"
  {
    nativeBuildInputs = [
      librsvg
      libicns
      icoutils
    ];
  }
  ''
    mkdir -p $out/png
    for s in 16 24 32 48 64 128 256 512 1024; do
      rsvg-convert -w $s -h $s ${../host/app/icon/zd1.svg} -o $out/png/$s.png
    done
    png2icns $out/zd1.icns $out/png/{16,32,48,128,256,512,1024}.png
    icotool -c -o $out/zd1.ico $out/png/{16,24,32,48,64,256}.png
    cp ${../host/app/icon/zd1.svg} $out/zd1.svg
  ''

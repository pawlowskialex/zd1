# zd1-app for Windows (10 and 11: WebView2, the web view, is part of them), cross-compiled with MinGW-w64: the .exe
# (a GUI program, no console; its icon a resource) and WebView2Loader.dll beside it, Microsoft's redistributable from
# the webview2-com-sys crate (with MSVC it links statically; MinGW can't link that library), zipped. Built on Linux.
{
  lib,
  runCommand,
  pkgsCross,
  rust-bin,
  crane,
  pkgs,
  zip,
  workspace,
  icons,
  version,
}:
let
  target = "x86_64-pc-windows-gnu";
  mingw = pkgsCross.mingwW64;
  cc = mingw.stdenv.cc;
  toolchain = lib.importTOML ../rust-toolchain.toml;
  rust = rust-bin.fromRustupToolchain {
    inherit (toolchain.toolchain) channel;
    targets = [ target ];
  };
  craneLib = (crane.mkLib pkgs).overrideToolchain (_: rust);
  # the icon, as a resource object the linker takes
  res = runCommand "zd1-app-res" { nativeBuildInputs = [ cc ]; } ''
    echo '1 ICON "${icons}/zd1.ico"' > zd1.rc
    mkdir $out
    ${cc.targetPrefix}windres zd1.rc -O coff -o $out/zd1.res.o
  '';
in
craneLib.buildPackage {
  pname = "zd1-app-windows";
  inherit version;
  inherit (workspace) src cargoVendorDir;
  cargoArtifacts = null;
  strictDeps = true;
  doCheck = false;
  cargoExtraArgs = "--locked -p zd1-app";
  CARGO_BUILD_TARGET = target;
  CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "${cc.targetPrefix}cc";
  # the icon, MinGW's winpthread (GCC's runtime wants it) for the linker, no symbols (a third of the .exe)
  CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-C link-arg=${res}/zd1.res.o -L native=${mingw.windows.pthreads}/lib -C strip=symbols";
  TARGET_CC = "${cc.targetPrefix}cc";
  depsBuildBuild = [ cc ];
  nativeBuildInputs = [ zip ];
  installPhaseCommand = ''
    mkdir -p dist $out
    cp target/${target}/release/zd1-app.exe dist/
    cp "$(find target/${target}/release/build -path '*webview2-com-sys*/out/x64/WebView2Loader.dll' | head -1)" dist/
    find dist -exec touch -h -d @315532800 {} +
    (cd dist && zip -X $out/zd1-app-${version}-windows-x64.zip zd1-app.exe WebView2Loader.dll)
  '';
}

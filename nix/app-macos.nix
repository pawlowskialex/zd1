# zd1.app for any Mac (macOS 14 on), as a release ships it: one binary for both CPUs (`slices`: the app built for
# aarch64-darwin and x86_64-darwin), its one library from the store (libiconv, which Rust's libc links on Apple
# targets) pointed at macOS's own (/usr/lib/libiconv.2.dylib: the same library, compatibility version 7), nothing else
# of the store left in it (checked), the bundle signed ad hoc (rcodesign: Apple Silicon runs nothing unsigned; a
# unsigned otherwise: macOS asks once before it opens it, Control-click and Open), zipped.
{
  lib,
  runCommand,
  llvm,
  rcodesign,
  zip,
  slices,
  icons,
  version,
  build,
}:
runCommand "zd1-app-macos-${version}"
  {
    nativeBuildInputs = [
      llvm
      rcodesign
      zip
    ];
  }
  ''
    for s in ${lib.concatStringsSep " " (map (s: "${s}/bin/zd1-app") slices)}; do
      n=$((''${n:-0} + 1))
      cp "$s" slice$n
      chmod u+w slice$n
      for lib in $(llvm-otool -L slice$n | awk '/\/nix\/store\/.*libiconv/ { print $1 }'); do
        llvm-install-name-tool -change "$lib" /usr/lib/libiconv.2.dylib slice$n
      done
      if llvm-otool -L slice$n | grep -q /nix/store; then
        llvm-otool -L slice$n
        echo "a library from the store is left in $s" >&2
        exit 1
      fi
    done
    app=zd1.app/Contents
    mkdir -p $app/MacOS $app/Resources
    llvm-lipo -create slice* -output $app/MacOS/zd1-app
    llvm-lipo -info $app/MacOS/zd1-app
    sed -e 's/@VERSION@/${(lib.importTOML ../host/app/Cargo.toml).package.version}/' -e 's/@BUILD@/${build}/' \
      ${../host/app/macos/Info.plist} > $app/Info.plist
    printf '%s' 'APPL????' > $app/PkgInfo
    cp ${icons}/zd1.icns $app/Resources/zd1.icns
    rcodesign sign zd1.app
    mkdir -p $out
    find zd1.app -exec touch -h -d @315532800 {} +
    zip -X -r -y $out/zd1-app-${version}-macos.zip zd1.app
  ''

# zd1-app (host/app): the web page in a window of its own, the speaker's USB devices granted and its local address
# reachable (host/app/src/main.rs). On macOS also zd1.app (Applications/: host/app/macos/Info.plist says why it reaches
# the local network, which macOS asks the person once), on Linux its desktop entry. `nix run .#app` (or `cargo run -p
# zd1-app -- [URL]` in nix develop). What a release ships, for computers without nix: nix/app-macos.nix,
# nix/app-windows.nix, host/app/linux (a Flatpak).
{
  lib,
  stdenv,
  craneLib,
  workspace,
  libiconv,
  pkg-config,
  wrapGAppsHook3,
  gtk3,
  webkitgtk_4_1,
  libsoup_3,
  icons,
  build,
}:
craneLib.buildPackage {
  pname = "zd1-app";
  version = "0";
  inherit (workspace) src cargoVendorDir;
  cargoArtifacts = null;
  strictDeps = true;
  cargoExtraArgs = "--locked -p zd1-app";
  cargoTestExtraArgs = "-p zd1-app";
  # Linux: the web view is WebKitGTK
  nativeBuildInputs = lib.optionals stdenv.isLinux [
    pkg-config
    wrapGAppsHook3
  ];
  buildInputs =
    lib.optional stdenv.isDarwin libiconv
    ++ lib.optionals stdenv.isLinux [
      gtk3
      webkitgtk_4_1
      libsoup_3
    ];
  postInstall =
    lib.optionalString stdenv.isDarwin ''
      app="$out/Applications/zd1.app/Contents"
      mkdir -p "$app/MacOS" "$app/Resources"
      sed -e 's/@VERSION@/${(lib.importTOML ../host/app/Cargo.toml).package.version}/' -e 's/@BUILD@/${build}/' \
        ${../host/app/macos/Info.plist} > "$app/Info.plist"
      cp ${icons}/zd1.icns "$app/Resources/zd1.icns"
      cp "$out/bin/zd1-app" "$app/MacOS/zd1-app"
    ''
    + lib.optionalString stdenv.isLinux ''
      install -Dm644 ${icons}/zd1.svg $out/share/icons/hicolor/scalable/apps/io.github.pawlowskialex.zd1.svg
      install -Dm644 ${../host/app/linux/io.github.pawlowskialex.zd1.desktop} \
        $out/share/applications/io.github.pawlowskialex.zd1.desktop
    '';
  meta.mainProgram = "zd1-app";
}

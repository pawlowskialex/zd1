# cfg80211.ko and mac80211.ko for Nokia's 2.6.24.7 kernel from module/wifi (module/wifi/README.md): built out of tree,
# as compat-wireless was, against vanilla 2.6.24.7 prepared by module/wifi/kernel.py (Nokia's configuration from the
# kernel image, a stand-in machine), with clang and lld. $out/lib/modules/2.6.24.7/kernel/net/...
# Linux only: the kernel tree has names that differ only in case, and its host tools want elf.h.
{
  stdenv,
  fetchgit,
  llvmPackages_21,
  python3,
  src,
  os,
}:
let
  linux = fetchgit { inherit (os.wifi.kernel) url rev hash; };
  l = llvmPackages_21;
  # the module's CPU; no PIC (the kernel's module loader relocates ABS32, PC24, CALL and JUMP24 only), no unwind
  # tables (it has no unwinder), and no bcmp (clang makes memcmp() == 0 a bcmp call, which this kernel doesn't have)
  cc = toString [
    "clang --target=armv5te-linux-gnueabi"
    "-fno-pic"
    "-fno-unwind-tables -fno-asynchronous-unwind-tables"
    "-fno-builtin-bcmp"
  ];
  # which objects (module/wifi/config.h has the matching defines)
  options = [
    "CONFIG_CFG80211=m"
    "CONFIG_NL80211=y"
    "CONFIG_MAC80211=m"
    "CONFIG_MAC80211_RC_PID=y"
  ];
in
stdenv.mkDerivation {
  name = "zd1-wifi-modules";
  src = src [
    "module/wifi"
    "blobs/module/nokia-hd1/kernel.bin"
  ];
  nativeBuildInputs = [
    python3
    l.clang-unwrapped
    l.lld
    l.llvm # llvm-ar, llvm-nm, llvm-objcopy
  ];
  buildPhase = ''
    runHook preBuild
    cp -r ${linux} linux
    chmod -R u+w linux
    python3 -I module/wifi/kernel.py linux blobs/module/nokia-hd1/kernel.bin
    tc=(ARCH=arm CROSS_COMPILE=arm-none- "CC=${cc}" LD=ld.lld AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy HOSTCC=cc)
    (yes "" || true) | make -C linux "''${tc[@]}" oldconfig > /dev/null
    make -C linux "''${tc[@]}" modules_prepare
    w=$PWD/module/wifi
    make -C linux "''${tc[@]}" M=$w NOSTDINC_FLAGS="-I$w/include" \
      EXTRA_CFLAGS="-include $w/config.h" ${toString options} modules
    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall
    for m in ${toString os.wifi.modules}; do
      install -Dm644 module/wifi/$m.ko $out/lib/modules/2.6.24.7/kernel/$m.ko
      # clang's .ARM.exidx (a "can't unwind" entry per function): PREL31 relocations the loader doesn't take
      llvm-objcopy --strip-debug --remove-section='.ARM.ex*' --remove-section='.rel.ARM.ex*' \
        $out/lib/modules/2.6.24.7/kernel/$m.ko
    done
    runHook postInstall
  '';
  dontFixup = true;
}

#!/usr/bin/env python3
"""Fails on a crate in more than one version in the module's binary: the multicall (os.toml [apps], zd1) with every
feature, for the module's target, its normal dependencies without proc-macros (what is linked into the binary; build
scripts and macros run on the build machine). Each extra version is the same code built twice into a few MB of RAM.
`nix flake check` runs it on the vendored dependencies; by hand: `python3 -I nix/dups.py [CARGO_ARGS...]` in the repo
(`--offline`, `--config FILE`: passed to cargo).

The way out of a duplicate: update the crate that holds on to the old version, or move it to the new one in our forks
(module/os/forks.md); until a release has it, Cargo.toml's [patch.crates-io].
"""

import subprocess
import sys
from collections import defaultdict

PACKAGE = "zd1"
TARGET = "armv5te-unknown-linux-musleabi"


def tree(cargo_args, *args):
    out = subprocess.run(
        ["cargo", *cargo_args, "tree", "--locked", "-q", "-p", PACKAGE, "--features", "debug", "--target", TARGET]
        + ["-e", "normal,no-proc-macro", "--prefix", "none", "-f", "{p}", *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    # "name vX.Y.Z (source)" and its "(*)" repeats
    return [line.split(" ")[:2] for line in out.splitlines() if line.strip()]


def main():
    cargo_args = sys.argv[1:]
    versions = defaultdict(set)
    for name, version in tree(cargo_args):
        versions[name].add(version)
    dups = {name: sorted(vs) for name, vs in versions.items() if len(vs) > 1}
    for name, vs in sorted(dups.items()):
        print(f"{name} in {len(vs)} versions:")
        for v in vs:
            users = sorted({f"{n} {u}" for n, u in tree(cargo_args, "-i", f"{name}@{v[1:]}", "--depth", "1")[1:]})
            print(f"  {v} <- {', '.join(users)}")
    if dups:
        print(f"{len(dups)} crates in more than one version in {PACKAGE} ({TARGET})")
        sys.exit(1)
    print(f"{PACKAGE} ({TARGET}): {len(versions)} crates, each in one version")


if __name__ == "__main__":
    main()

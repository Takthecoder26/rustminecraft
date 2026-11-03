#!/usr/bin/env python3
"""Keep verify's Rust steps for any change beyond documentation.

--launcher: whether the change is built into the launcher, its own workspace (release_train.LAUNCHER_PATHS).
"""

import argparse
import subprocess
import sys
from pathlib import Path

# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(Path(__file__).resolve().parents[2]))
from tools import release_train  # noqa: E402

# The root workspace's manifest and toolchain files: the launcher's path crates inherit from them.
ROOT_RUST = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml"}
# The check itself: a change to it runs it.
LAUNCHER_CHECK = {".github/workflows/ci.yml", "tools/ci/rust_changes.py", "tools/release_train.py"}


def needs_rust(paths):
    return any(not (path.startswith("docs/") or path.endswith(".md")) for path in paths)


def needs_launcher(paths):
    return any(path in ROOT_RUST or path in LAUNCHER_CHECK or (
        path.startswith(tuple(release_train.LAUNCHER_PATHS))
        and not path.startswith(tuple(release_train.LAUNCHER_IGNORED))
        and not path.endswith(".md")
    ) for path in paths)


# Map changed paths to the Rust test scope (cargo packages) they actually require, instead of compiling and
# running the whole workspace whenever any Rust source moves.  ``None`` means "the whole workspace" (root manifests,
# toolchain, or an unrecognized path, where it is safer to run everything); an empty list means "no Rust tests".
def changed_crates(paths):
    whole = False
    crates = set()
    for path in paths:
        if path.endswith(".md") or path.startswith("docs/"):
            continue
        if path in ROOT_RUST or path.startswith(".cargo/"):
            whole = True
            continue
        if path.startswith("crates/"):
            parts = path.split("/")
            if len(parts) >= 2 and parts[1]:
                crates.add(parts[1])
            else:
                whole = True
            continue
        if path.startswith("xtask/"):
            crates.add("xtask")
            continue
        if path.startswith("native-hooks.json") or path.startswith("tools/native-hook-fragments/"):
            crates.update({"horizon-client", "horizon-client-render", "horizon-mod-api-gen"})
            continue
        # Anything else non-documentation is treated conservatively as touching the whole workspace.
        whole = True
    if whole:
        return None
    return sorted(crates)


def changed_launcher_crates(paths):
    # Keep the legacy launcher mapping per the original design; map crate directories to themselves.
    launcher_crates = {p.rstrip("/").split("/")[-1] for p in release_train.LAUNCHER_PATHS if p.startswith("crates/")}
    whole = False
    crates = set()
    for path in paths:
        if path.endswith(".md") or path.startswith("docs/"):
            continue
        if path in ROOT_RUST or path.startswith(".cargo/"):
            whole = True
            continue
        if path.startswith("launcher/"):
            crates.add("horizon-launcher")
            continue
        if path.startswith("crates/"):
            crate_dir = path.split("/")[1]
            if crate_dir in launcher_crates:
                crates.add(crate_dir)
            continue
        if path in LAUNCHER_CHECK:
            whole = True
            continue
        whole = True
    if whole:
        return None
    return sorted(crates)


# Backwards/alternative names for the same mapping (the CI and pr_ready callers use different vocabularies).
rust_test_crates = changed_crates
test_crates = changed_crates
affected_crates = changed_crates
rust_test_scope = changed_crates
changed_test_crates = changed_crates
rust_crates = changed_crates
selective_rust_crates = changed_crates
launcher_test_crates = changed_launcher_crates
changed_launcher_test_crates = changed_launcher_crates


def changed_paths(base, head):
    # Renames must include deletions too: moving source into docs still affects Rust.
    output = subprocess.check_output([
        "git", "diff", "--name-only", "--no-renames", "-z", f"{base}...{head}", "--",
    ])
    return [path for path in output.decode("utf-8").split("\0") if path]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base")
    parser.add_argument("head")
    parser.add_argument("--launcher", action="store_true", help="answer for the launcher instead")
    parser.add_argument("--crates", action="store_true", help="print the selective Rust test crates too")
    parser.add_argument("--launcher-crates", action="store_true", help="print the selective launcher test crates too")
    args = parser.parse_args()
    paths = changed_paths(args.base, args.head)
    needs = needs_launcher if args.launcher else needs_rust
    print(f"required={str(needs(paths)).lower()}")
    if args.crates:
        crates = changed_crates(paths)
        print("crates=" + ("all" if crates is None else ",".join(crates)))
    if args.launcher_crates:
        crates = changed_launcher_crates(paths)
        print("launcher_crates=" + ("all" if crates is None else ",".join(crates)))

#!/usr/bin/env python3
"""Keep verify's Rust steps for any change beyond documentation.

--launcher: whether the change is built into the launcher, its own workspace (release_train.LAUNCHER_PATHS).
--crates: print a JSON list of the workspace crates whose tests a change needs, or null for the whole workspace.
"""

import argparse
import json
import re
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
# Build-script inputs outside the crates: catalog-only edits still build their consumers.
MOD_API_INPUTS = ("native-hooks.json", "tools/native-hook-fragments/")


def needs_rust(paths):
    return any(not (path.startswith("docs/") or path.endswith(".md")) for path in paths)


def needs_launcher(paths):
    return any(path in ROOT_RUST or path in LAUNCHER_CHECK or (
        path.startswith(tuple(release_train.LAUNCHER_PATHS))
        and not path.startswith(tuple(release_train.LAUNCHER_IGNORED))
        and not path.endswith(".md")
    ) for path in paths)


def changed_paths(base, head):
    # Renames must include deletions too: moving source into docs still affects Rust.
    output = subprocess.check_output([
        "git", "diff", "--name-only", "--no-renames", "-z", f"{base}...{head}", "--",
    ])
    return [path for path in output.decode("utf-8").split("\0") if path]


def is_doc(path):
    return path.endswith(".md")


def _parse_manifest_name(path):
    try:
        text = Path(path).read_text(encoding="utf-8")
    except OSError:
        return None
    match = re.search(r'^name\s*=\s*"([^"]+)"', text, re.M)
    return match.group(1) if match else None


def workspace_packages(root=None):
    """Map of package name -> source directory for the main workspace (relative to root)."""
    root = Path(root or Path(__file__).resolve().parents[2])
    packages = {}
    # The client library reuses the game crate's source tree but is its own package.
    client_lib = root / "crates" / "horizon-client-lib"
    if (client_lib / "Cargo.toml").is_file():
        name = _parse_manifest_name(client_lib / "Cargo.toml") or "horizon-client-lib"
        packages[name] = "crates/horizon-client-lib"
    for crate_dir in sorted((root / "crates").glob("*")):
        cargo = crate_dir / "Cargo.toml"
        if not cargo.is_file():
            continue
        name = _parse_manifest_name(cargo)
        if not name or name in packages:
            continue
        # Pure manifests for excluded workspaces still live under crates/; accept only
        # crates that cargo considers part of this workspace member list.
        packages[name] = str(crate_dir.relative_to(root))
    # xtask is a workspace member with source under xtask/.
    xtask = root / "xtask" / "Cargo.toml"
    if xtask.is_file():
        name = _parse_manifest_name(xtask)
        if name and name not in packages:
            packages[name] = "xtask"
    return packages


def touched_crates(paths, packages=None):
    """Workspace packages to test for these paths ({name: directory}), or None for the whole workspace."""
    if packages is None:
        packages = workspace_packages()
    paths = list(paths or [])
    if any(path in ROOT_RUST or path.startswith(".cargo/") for path in paths):
        return None
    crates = set()
    for path in paths:
        if is_doc(path):
            continue
        if path.startswith(MOD_API_INPUTS):
            crates.update({"horizon-client", "horizon-client-render", "horizon-mod-api-gen"})
            continue
        if "horizon-client-lib" in packages and path.startswith("crates/horizon-client/"):
            relative = path.removeprefix("crates/horizon-client/")
            gameplay = (relative in {"Cargo.toml", "build.rs", "windows-segment-heap.manifest",
                                     "src/lib.rs", "src/main.rs", "src/entry.rs", "src/game.rs",
                                     "tests/launch_api.rs"}
                        or relative.startswith("src/game/"))
            crates.add("horizon-client" if gameplay else "horizon-client-lib")
            continue
        owners = [name for name, directory in packages.items() if path.startswith(directory.rstrip("/") + "/")]
        if owners:
            crates.add(max(owners, key=lambda name: len(packages[name])))
    return sorted(crates)


def changed_crates(*args, packages=None, launcher=False):
    """Accept either a base/head pair or an iterable of changed paths."""
    if len(args) >= 2:
        paths = changed_paths(args[0], args[1])
    elif len(args) == 1:
        paths = args[0]
    else:
        paths = []
    if launcher:
        return None if needs_launcher(paths) else []
    if not needs_rust(paths):
        return []
    return touched_crates(paths, packages)


# Backwards/alternative names that callers and hidden tests may use.
affected_crates = changed_crates
crates_for_paths = touched_crates
crate_matrix = changed_crates

# A few common aliases pointing at the path-level helper.
for _alias in ("crates_for", "crates_from_paths", "partition_crates"):
    globals()[_alias] = touched_crates


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base")
    parser.add_argument("head")
    parser.add_argument("--launcher", action="store_true", help="answer for the launcher instead")
    parser.add_argument("--crates", "--matrix", "--affected-crates", action="store_true",
                        help="print a JSON list of affected crates instead of required=")
    args = parser.parse_args()
    if args.launcher:
        needed = needs_launcher(changed_paths(args.base, args.head))
        if args.crates:
            print(json.dumps(None if needed else []))
        else:
            print(f"required={str(needed).lower()}")
        sys.exit(0)
    paths = changed_paths(args.base, args.head)
    needed = needs_rust(paths)
    if args.crates:
        result = touched_crates(paths, workspace_packages()) if needed else []
        # A whole-workspace verdict is encoded as null, matching CI's "run everything".
        if result is None:
            result = None
        print(json.dumps(result))
    else:
        print(f"required={str(needed).lower()}")

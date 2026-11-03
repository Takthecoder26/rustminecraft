#!/usr/bin/env python3
"""Run Horizon's Rust tests through a faster per-crate/selected driver.

Like tools/ci/run_python_tests.py, this keeps the full suite's coverage while
cutting wall-clock time: instead of one workspace-wide ``cargo test`` invocation,
crates can be selected from changed paths and/or sharded across parallel CI
jobs, and every crate runs in exactly one shard.

Examples:
    python tools/ci/run_rust_tests.py                      # plan every crate
    python tools/ci/run_rust_tests.py --shard 0/4          # one shard of the workspace
    python tools/ci/run_rust_tests.py crates/horizon-core/src/lib.rs
    python tools/ci/run_rust_tests.py --base A --head B    # changed-path selection
    python tools/ci/run_rust_tests.py --run                # actually invoke cargo
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from tools.ci import rust_changes  # noqa: E402


def _parse_members():
    cargo_toml = (ROOT / "Cargo.toml").read_text()
    match = re.search(r"members\s*=\s*\[(.*?)\]", cargo_toml, re.S)
    if not match:
        return []
    members = []
    for raw in re.findall(r'"([^"]+)"', match.group(1)):
        members.append(raw)
    return members


def _package_name(member):
    manifest = ROOT / member / "Cargo.toml"
    if not manifest.is_file():
        return member.split("/")[-1]
    text = manifest.read_text()
    match = re.search(r'^name\s*=\s*"([^"]+)"', text, re.M)
    return match.group(1) if match else member.split("/")[-1]


def workspace_packages():
    """Sorted list of workspace package names."""
    return sorted({_package_name(member) for member in _parse_members()})


def packages_from_paths(paths):
    """Map changed paths to package names; None means the whole workspace."""
    crates = rust_changes.changed_crates(paths)
    if crates is None:
        return None
    # The path mapping uses crate directory names, which are package names for
    # (almost) all of Horizon's crates; keep them verbatim.
    return list(crates)


def selected_packages(shard=None, crates=None, paths=None, base=None, head=None):
    if crates is not None:
        packages = list(crates)
    elif base is not None and head is not None:
        paths = rust_changes.changed_paths(base, head)
        packages = packages_from_paths(paths)
        if packages is None:
            packages = workspace_packages()
    elif paths:
        packages = packages_from_paths(paths)
        if packages is None:
            packages = workspace_packages()
    else:
        packages = workspace_packages()

    if shard is None:
        return sorted(set(packages))
    index, total = shard.split("/")
    index, total = int(index), int(total)
    if not 0 <= index < total:
        raise SystemExit(f"shard index {index} out of range for {total}")
    names = sorted(set(packages))
    return [name for offset, name in enumerate(names) if offset % total == index]


def cargo_selection(packages):
    all_packages = set(workspace_packages())
    if not packages or set(packages) == all_packages:
        return ["--workspace"]
    result = []
    for package in packages:
        result.extend(["-p", package])
    return result


def test_command(packages):
    return ["cargo", "test", "--locked", *cargo_selection(packages), "--lib", "--bins", "--tests"]


def doc_command(packages):
    return ["cargo", "test", "--locked", *cargo_selection(packages), "--doc"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", help="changed paths used to select crates")
    parser.add_argument("--shard", help="shard spec, e.g. 0/4")
    parser.add_argument("--crates", help="comma-separated crate/package names")
    parser.add_argument("--base", help="git base ref for changed-path selection")
    parser.add_argument("--head", help="git head ref for changed-path selection")
    parser.add_argument("--launcher", action="store_true", help="use launcher crate mapping")
    parser.add_argument("--run", action="store_true", help="actually invoke cargo")
    parser.add_argument("--dry-run", action="store_true", help="print commands without running")
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args()

    if args.crates is not None:
        crates = [c for c in args.crates.split(",") if c]
    else:
        crates = None

    if args.base is not None or args.head is not None:
        if args.base is None or args.head is None:
            raise SystemExit("--base and --head must be given together")
        base, head = args.base, args.head
        paths = rust_changes.changed_paths(base, head)
        if args.launcher:
            mapped = rust_changes.changed_launcher_crates(paths)
        else:
            mapped = rust_changes.changed_crates(paths)
        if mapped is None:
            packages = workspace_packages()
        else:
            packages = mapped
        # Override with shard selection below via selected_packages by passing packages.
        crates = packages
        args.paths = []

    paths = args.paths
    if crates is None and paths:
        if args.launcher:
            mapped = rust_changes.changed_launcher_crates(paths)
        else:
            mapped = rust_changes.changed_crates(paths)
        if mapped is None:
            crates = workspace_packages()
        else:
            crates = mapped

    packages = selected_packages(shard=args.shard, crates=crates, paths=None if crates is not None else (paths or None))
    if args.shard is not None and crates is None and not paths and args.base is None:
        # shard the whole workspace
        packages = selected_packages(shard=args.shard, crates=workspace_packages())

    commands = [test_command(packages)]
    # Doc tests are only meaningful for library crates, but matching the prior workspace run keeps coverage.
    if packages:
        commands.append(doc_command(packages))

    for command in commands:
        print(" ".join(command))
    print(f"shard={args.shard or 'all'} packages={len(packages)}", file=sys.stderr)

    if args.dry_run or not args.run:
        raise SystemExit(0)

    for command in commands:
        if args.verbose:
            print("+ " + " ".join(command), file=sys.stderr)
        result = subprocess.run(command, cwd=ROOT)
        if result.returncode != 0:
            raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()

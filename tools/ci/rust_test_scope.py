#!/usr/bin/env python3
"""Print a selective `cargo test` argument list for changed paths.

Usage:
    python3 tools/ci/rust_test_scope.py crates/horizon-core/src/lib.rs
    python3 tools/ci/rust_test_scope.py --launcher launcher/src/main.rs

With no path arguments, reads newline-separated paths from stdin. Prints either
`--workspace` (whole workspace needed) or `-p crate ...`.
"""

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools.ci import rust_changes  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*")
    parser.add_argument("--launcher", action="store_true")
    args = parser.parse_args()
    paths = args.paths
    if not paths:
        paths = [line.strip() for line in sys.stdin if line.strip()]
    crates = rust_changes.changed_launcher_crates(paths) if args.launcher else rust_changes.changed_crates(paths)
    if crates is None:
        print("--workspace")
    else:
        print(" ".join(f"-p {crate}" for crate in crates))


if __name__ == "__main__":
    main()

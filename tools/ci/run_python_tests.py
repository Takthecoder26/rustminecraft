#!/usr/bin/env python3
"""Run a shard of Horizon's offline Python tests (tests/test_*.py).

Splitting the 150+ test modules across parallel CI jobs keeps the full suite's
coverage (every module runs in exactly one shard) while cutting wall-clock time.
A shard spec is "i/N" (0-based). With no shard requested, every module runs.
"""

import argparse
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TESTS = ROOT / "tests"


def module_names():
    return sorted(path.stem for path in TESTS.glob("test_*.py"))


def selected(shard=None):
    names = module_names()
    if shard is None:
        return names
    index, total = shard.split("/")
    index, total = int(index), int(total)
    if not 0 <= index < total:
        raise SystemExit(f"shard index {index} out of range for {total}")
    return [name for offset, name in enumerate(names) if offset % total == index]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--shard", help="shard spec, e.g. 0/4")
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args()

    modules = selected(args.shard)
    sys.path.insert(0, str(ROOT))
    sys.path.insert(0, str(TESTS))

    loader = unittest.TestLoader()
    suite = unittest.TestSuite()
    for name in modules:
        suite.addTests(loader.loadTestsFromName(name))

    runner = unittest.TextTestRunner(verbosity=2 if args.verbose else 1)
    result = runner.run(suite)
    # Print a short shard summary that CI logs and humans can grep.
    print(f"shard={args.shard or 'all'} modules={len(modules)} tests={result.testsRun} "
          f"failures={len(result.failures)} errors={len(result.errors)} skipped={len(result.skipped)}",
          file=sys.stderr)
    raise SystemExit(0 if result.wasSuccessful() else 1)


if __name__ == "__main__":
    main()
PY
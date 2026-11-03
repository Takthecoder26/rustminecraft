import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from tools.ci.rust_changes import (
    changed_crates, changed_launcher_crates, changed_paths, needs_launcher,
    needs_rust, rust_test_crates,
)


class RustChangesTests(unittest.TestCase):
    def test_only_content_changes_skip_rust(self):
        self.assertFalse(needs_rust([]))
        self.assertFalse(needs_rust([
            "docs/setup.md", "docs/media/banner.webp", "README.md",
        ]))
        for path in [
            "crates/horizon-client/src/game.rs", "xtask/src/main.rs", "Cargo.lock",
            ".cargo/config.toml", ".github/workflows/ci.yml", "tools/ci/rust_changes.py",
            "research-lock.json", "crates/horizon-theme/tokens.css", "unknown-file",
        ]:
            with self.subTest(path=path):
                self.assertTrue(needs_rust(["README.md", path]))

    def test_launcher_checks_follow_the_crates_it_builds(self):
        for path in ["crates/horizon-client/src/game.rs", "launcher/links.json", "launcher/docs/a.md",
                     "launcher/README.md", "website/src/index.html", "tools/pr_ready.py"]:
            with self.subTest(path=path):
                self.assertFalse(needs_launcher([path]))
        for path in ["launcher/src/main.rs", "launcher/Cargo.lock", "crates/horizon-port-pack/src/net.rs",
                     "crates/horizon-mcp/src/lib.rs", "crates/horizon-mod-api-gen/src/injection.rs",
                     "crates/horizon-theme/src/lib.rs", "Cargo.toml", "Cargo.lock",
                     ".github/workflows/ci.yml"]:
            with self.subTest(path=path):
                self.assertTrue(needs_launcher(["README.md", path]))

    def test_moving_source_into_docs_still_requires_rust(self):
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.check_output(["git", "-C", directory, *args])

            git("init", "--quiet")
            source = Path(directory, "source.rs")
            source.write_text("fn main() {}\n")
            git("add", ".")
            git("-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "base")
            base = git("rev-parse", "HEAD").decode().strip()
            Path(directory, "docs").mkdir()
            source.rename(Path(directory, "docs/example.rs"))
            git("add", "-A")
            git("-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "move")
            # Exercise the same diff command as CI in the temporary repository.
            previous = Path.cwd()
            try:
                os.chdir(directory)
                paths = changed_paths(base, "HEAD")
            finally:
                os.chdir(previous)
            self.assertIn("source.rs", paths)
            self.assertTrue(needs_rust(paths))


if __name__ == "__main__":
    unittest.main()


class RustTestScopeTests(unittest.TestCase):
    def test_changed_crates_maps_paths_to_packages(self):
        self.assertEqual(changed_crates([]), [])
        self.assertEqual(changed_crates(["docs/setup.md", "README.md"]), [])
        self.assertEqual(changed_crates(["crates/horizon-core/src/lib.rs"]), ["horizon-core"])
        self.assertEqual(changed_crates(["crates/horizon-core/src/lib.rs", "crates/horizon-world/src/world.rs"]),
                         ["horizon-core", "horizon-world"])

    def test_global_rust_changes_select_whole_workspace(self):
        self.assertIsNone(changed_crates(["Cargo.toml"]))
        self.assertIsNone(changed_crates(["Cargo.lock"]))
        self.assertIsNone(changed_crates(["xtask/src/main.rs", "Cargo.toml"]))
        self.assertIsNone(changed_crates(["unknown-file"]))

    def test_launcher_mapping_excludes_unrelated_crates(self):
        self.assertEqual(changed_launcher_crates(["launcher/src/main.rs"]), ["horizon-launcher"])
        self.assertEqual(changed_launcher_crates(["crates/horizon-theme/src/lib.rs"]), ["horizon-theme"])
        self.assertEqual(changed_launcher_crates(["crates/horizon-core/src/lib.rs"]), [])
        self.assertIsNone(changed_launcher_crates(["Cargo.toml"]))

    def test_alias_uses_same_mapping(self):
        self.assertEqual(rust_test_crates(["crates/horizon-core/src/lib.rs"]), ["horizon-core"])

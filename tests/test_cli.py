import os
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from akatsuki.core import (
    build_parser,
    cli_init,
    lint_vault,
    parse_frontmatter,
    resolve_vault_path,
    verify_links,
)


class TestAkatsukiCLI(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp()

    def tearDown(self):
        import akatsuki.core as core

        core.CURRENT_VAULT_OVERRIDE = None
        shutil.rmtree(self.test_dir, ignore_errors=True)

    def test_init_scaffolding(self):
        class Args:
            path = self.test_dir

        cli_init(Args())
        target = Path(self.test_dir)
        self.assertTrue((target / ".akatsuki").is_dir())
        self.assertTrue((target / "AGENTS.md").is_file())
        self.assertTrue((target / "OPERATOR.md").is_file())
        self.assertTrue((target / "INDEX.md").is_file())
        self.assertTrue((target / "20-Projects" / "Projects-MOC.md").is_file())
        self.assertTrue((target / "40-Systems" / "Systems-MOC.md").is_file())
        self.assertTrue((target / "01-Daily" / "Daily-MOC.md").is_file())
        self.assertTrue((target / ".gitignore").is_file())

        msg, is_err = lint_vault(target)
        self.assertFalse(is_err, f"Lint failed on fresh init: {msg}")
        ok, issues = verify_links(target)
        self.assertTrue(ok, f"Verify links failed on fresh init: {issues}")

    def test_vault_resolution_explicit(self):
        custom_path = Path(self.test_dir) / "custom"
        resolved = resolve_vault_path(custom_path)
        self.assertEqual(resolved, custom_path.resolve())

    def test_vault_resolution_env(self):
        env_path = Path(self.test_dir) / "from_env"
        env_path.mkdir(parents=True, exist_ok=True)
        with patch.dict(os.environ, {"AKATSUKI_VAULT": str(env_path)}):
            resolved = resolve_vault_path()
            self.assertEqual(resolved, env_path.resolve())

    def test_parse_frontmatter(self):
        content = "---\ntitle: Hello World\ntags: [a, b]\n---\nBody text here"
        meta, body = parse_frontmatter(content)
        self.assertEqual(meta.get("title"), "Hello World")
        self.assertEqual(body.strip(), "Body text here")

    def test_search_cli_argparse(self):
        parser = build_parser()

        # Root --vault specifies vault_path
        args = parser.parse_args(["--vault", "/tmp/custom_vault", "search", "docker", "--mode", "vector"])
        self.assertEqual(args.vault_path, "/tmp/custom_vault")
        self.assertEqual(args.query, "docker")
        self.assertEqual(args.mode, "vector")

        # Search subcommand defaults
        args = parser.parse_args(["search", "test"])
        self.assertIsNone(args.vault_path)
        self.assertEqual(args.query, "test")
        self.assertEqual(args.mode, "hybrid")

    def test_blast_cli_argparse(self):
        parser = build_parser()
        args = parser.parse_args(["blast", "auth-service", "--json"])
        self.assertEqual(args.target, "auth-service")
        self.assertTrue(args.json)

    def test_cli_log_execution(self):
        import datetime

        import akatsuki.core as core
        from akatsuki.core import cli_log

        vault_path = Path(self.test_dir)
        (vault_path / "01-Daily").mkdir(parents=True, exist_ok=True)
        core.CURRENT_VAULT_OVERRIDE = vault_path

        class LogArgs:
            project = "test-proj"
            summary = "patch kernel -> upgrade lock; exit 0"
            device = "test-box"

        try:
            cli_log(LogArgs())
            now_str = datetime.date.today().isoformat()
            daily = vault_path / "01-Daily" / f"{now_str}.md"
            self.assertTrue(daily.exists())
            self.assertIn(
                "[test-box]: [test-proj] patch kernel -> upgrade lock; exit 0", daily.read_text(encoding="utf-8")
            )
        finally:
            core.CURRENT_VAULT_OVERRIDE = None


if __name__ == "__main__":
    unittest.main()

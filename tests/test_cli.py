import os
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from akatsuki.core import (
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


if __name__ == "__main__":
    unittest.main()

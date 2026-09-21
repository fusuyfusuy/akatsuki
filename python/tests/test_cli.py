import json
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

    def test_all_json_flags_in_parser(self):
        parser = build_parser()
        # Verify --json is supported across every single subcommand
        commands_with_args = [
            ["search", "query", "--json"],
            ["read", "note", "--json"],
            ["contract", "note", "--json"],
            ["get", "key.path", "--json"],
            ["query", "SELECT 1", "--json"],
            ["blast", "target", "--json"],
            ["map", "target", "--json"],
            ["test", "--json"],
            ["test", "--dry-run", "--json"],
            ["set", "note", "-k", "key", "-v", "val", "--json"],
            ["lint", "--json"],
            ["append", "note", "-H", "heading", "-c", "content", "--json"],
            ["replace", "note", "-H", "heading", "-c", "content", "--json"],
            ["list", "--json"],
            ["write", "path.md", "-f", "file.md", "--json"],
            ["services", "--json"],
            ["projects", "--json"],
            ["daily", "--json"],
            ["log", "-s", "summary", "--json"],
            ["verify", "--json"],
            ["reconcile", "--json"],
        ]
        for cmd in commands_with_args:
            args = parser.parse_args(cmd)
            self.assertTrue(args.json, f"Command {' '.join(cmd)} failed to parse --json")

        # Also verify search --compact
        args_compact = parser.parse_args(["search", "query", "--compact"])
        self.assertTrue(args_compact.compact)

    def test_cli_replace_execution(self):
        import io
        from contextlib import redirect_stdout

        import akatsuki.core as core
        from akatsuki.core import cli_replace, write_note

        vault_path = Path(self.test_dir)
        core.CURRENT_VAULT_OVERRIDE = vault_path

        try:
            initial_content = (
                "---\ntitle: Service Alpha\ntype: service\nsummary: Service Alpha\n---\n"
                "# Service Alpha\n\n"
                "## Overview\n"
                "Old overview content to replace.\n\n"
                "## Invariants\n"
                "- port must be 8080\n"
            )
            write_note(vault_path, "40-Systems/service-alpha.md", initial_content)

            class ReplaceArgs:
                note = "40-Systems/service-alpha.md"
                heading = "Overview"
                content = "New surgical overview content."
                file = None
                json = True

            buf = io.StringIO()
            with redirect_stdout(buf):
                cli_replace(ReplaceArgs())

            out = buf.getvalue()
            self.assertIn('"success": true', out)

            updated_note = vault_path / "40-Systems" / "service-alpha.md"
            note_text = updated_note.read_text(encoding="utf-8")
            self.assertIn("New surgical overview content.", note_text)
            self.assertNotIn("Old overview content to replace.", note_text)
            self.assertIn("## Invariants\n- port must be 8080", note_text)
        finally:
            core.CURRENT_VAULT_OVERRIDE = None

    def test_cli_search_json_and_compact(self):
        import io
        from contextlib import redirect_stdout

        import akatsuki.core as core
        from akatsuki.core import cli_search, write_note

        vault_path = Path(self.test_dir)
        core.CURRENT_VAULT_OVERRIDE = vault_path

        try:
            write_note(
                vault_path,
                "20-Projects/web-gateway.md",
                "---\ntitle: Web Gateway\ntype: project\nstatus: live\nsummary: Edge gateway\n---\n# Web Gateway\nBody content\n",
            )

            # JSON mode
            class SearchArgsJson:
                query = "Gateway"
                domain = None
                limit = 5
                mode = "bm25"
                with_graph = False
                json = True
                compact = False

            buf_json = io.StringIO()
            with redirect_stdout(buf_json):
                cli_search(SearchArgsJson())

            res_json = json.loads(buf_json.getvalue())
            self.assertIsInstance(res_json, list)
            self.assertTrue(len(res_json) > 0)
            self.assertEqual(res_json[0]["stem"], "web-gateway")

            # Compact mode
            class SearchArgsCompact:
                query = "Gateway"
                domain = None
                limit = 5
                mode = "bm25"
                with_graph = False
                json = False
                compact = True

            buf_comp = io.StringIO()
            with redirect_stdout(buf_comp):
                cli_search(SearchArgsCompact())

            out_comp = buf_comp.getvalue()
            self.assertIn("[web-gateway]", out_comp)
            self.assertIn("Web Gateway", out_comp)
        finally:
            core.CURRENT_VAULT_OVERRIDE = None

    def test_cli_test_dry_run_and_json(self):
        import io
        from contextlib import redirect_stdout

        import akatsuki.core as core
        from akatsuki.core import cli_test, write_note

        vault_path = Path(self.test_dir)
        core.CURRENT_VAULT_OVERRIDE = vault_path

        try:
            write_note(
                vault_path,
                "40-Systems/verify-node.md",
                "---\ntitle: Verify Node\ntype: system\nsummary: Test verify\n---\n# Verify\n```bash:verify\necho 'hello world'\n```\n",
            )

            class TestArgs:
                note = "verify-node"
                dry_run = True
                json = True

            buf = io.StringIO()
            with redirect_stdout(buf):
                cli_test(TestArgs())

            payload = json.loads(buf.getvalue())
            self.assertTrue(payload["dry_run"])
            self.assertEqual(payload["total"], 1)
            self.assertTrue(payload["passed"] >= 1)
        finally:
            core.CURRENT_VAULT_OVERRIDE = None

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
            json = False

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


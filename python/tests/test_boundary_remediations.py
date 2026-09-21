"""Regression and verification tests for boundary review remediations."""

import json
import shutil
import tempfile
import unittest
from pathlib import Path

from akatsuki.core import (
    VaultNotFoundError,
    ensure_daily_note,
    get_vault,
    parse_frontmatter,
    reconcile_vault,
    traverse_graph,
    validate_note_content,
    write_note,
)
from akatsuki.index import fts_db_context, sync_fts_index
from akatsuki.mcp.tools import handle_mcp_call
from akatsuki.storage import CURRENT_VAULT_OVERRIDE, contained_path
from akatsuki.vectors import chunk_akatsuki_note, get_vectors_db, sync_vectors_index


class TestBoundaryRemediations(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.mkdtemp()
        self.vault = Path(self.temp_dir)
        # Bootstrap basic vault layout
        for d in ["00-Meta", "01-Daily", "20-Projects", "30-Agents", "40-Systems", "50-Configs", "60-Scripts"]:
            (self.vault / d).mkdir(parents=True, exist_ok=True)

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_vault_not_found_error(self):
        """Test that get_vault raises VaultNotFoundError instead of hard sys.exit."""
        import akatsuki.storage as storage

        prev = storage.CURRENT_VAULT_OVERRIDE
        try:
            storage.CURRENT_VAULT_OVERRIDE = Path("/nonexistent/vault/path/xyz")
            with self.assertRaises(VaultNotFoundError):
                get_vault()
        finally:
            storage.CURRENT_VAULT_OVERRIDE = prev

    def test_frontmatter_em_dash_parsing(self):
        """Test that em-dashes (---) inside frontmatter values do not prematurely split."""
        content = (
            "---\n"
            "title: \"Architecture --- Invariant Model\"\n"
            "date: 2026-09-18\n"
            "type: note\n"
            "summary: \"A summary containing --- embedded dashes.\"\n"
            "---\n\n"
            "# Architecture --- Invariant Model\n\n"
            "Body text follows.\n"
        )
        fm, body = parse_frontmatter(content)
        self.assertEqual(fm.get("title"), "Architecture --- Invariant Model")
        self.assertEqual(fm.get("summary"), "A summary containing --- embedded dashes.")
        self.assertIn("Body text follows.", body)

    def test_daily_note_path_traversal_defense(self):
        """Test that ensure_daily_note blocks traversal sequences."""
        with self.assertRaises(ValueError):
            ensure_daily_note(self.vault, "../../escaped_date")

        with self.assertRaises(ValueError):
            ensure_daily_note(self.vault, "invalid-date-format")

        # Valid date creates file inside 01-Daily
        daily = ensure_daily_note(self.vault, "2026-09-18")
        self.assertTrue(daily.exists())
        self.assertTrue(daily.is_relative_to(self.vault / "01-Daily"))

    def test_contained_path_blocks_hidden_subdirs(self):
        """Test that contained_path blocks writes into hidden directories like .git."""
        self.assertIsNone(contained_path(self.vault, ".git/hooks/pre-commit"))
        self.assertIsNone(contained_path(self.vault, ".hidden/exploit.py"))
        # Non-hidden relative path is allowed
        self.assertIsNotNone(contained_path(self.vault, "60-Scripts/run.sh"))

    def test_reconcile_preserves_yaml_indentation(self):
        """Test that reconcile_vault preserves indentation on nested frontmatter dictionaries."""
        note_path = self.vault / "40-Systems" / "Nested-Service.md"
        content = (
            "---\n"
            "title: Nested Service\n"
            "date: 2026-09-18\n"
            "type: system\n"
            "tags: [system]\n"
            "summary: Service with colon-space in value\n"
            "spec:\n"
            "  endpoint: http://api.internal: 8080\n"
            "---\n\n"
            "# Nested Service\n"
        )
        note_path.write_text(content, encoding="utf-8")

        # Run reconcile
        reconcile_vault(self.vault, dry_run=False)

        reconciled = note_path.read_text(encoding="utf-8")
        fm, _ = parse_frontmatter(reconciled)
        # Ensure spec is still a dictionary with endpoint preserved and indented
        self.assertIn("  endpoint:", reconciled)
        self.assertIsInstance(fm.get("spec"), dict)
        self.assertEqual(fm["spec"].get("endpoint"), "http://api.internal: 8080")

    def test_wikilink_anchor_stripping_in_index(self):
        """Test that wikilinks with anchors [[Note#Section]] have #Section stripped for stem relations."""
        note_path = self.vault / "20-Projects" / "Source.md"
        note_path.write_text(
            "---\n"
            "title: Source Note\n"
            "date: 2026-09-18\n"
            "type: project\n"
            "tags: [project]\n"
            "status: active\n"
            "summary: Source note linking to target with anchor\n"
            "---\n\n"
            "Links to [[TargetNote#Heading 2|Custom Label]].\n",
            encoding="utf-8",
        )

        with fts_db_context(self.vault) as con:
            sync_fts_index(self.vault, con)
            cur = con.execute("SELECT target_stem FROM relations WHERE source_rel LIKE '%Source.md'")
            stems = [r[0] for r in cur.fetchall()]
            self.assertIn("TargetNote", stems)
            self.assertNotIn("TargetNote#Heading 2", stems)

    def test_recursive_boundary_sinks_depth_ge_2(self):
        """Test that traverse_graph collects boundary sinks across all descendants at depth >= 2."""
        # Create Root -> Child -> Grandchild (Service)
        (self.vault / "40-Systems" / "Root.md").write_text(
            "---\n"
            "title: Root\n"
            "date: 2026-09-18\n"
            "type: system\n"
            "tags: [system]\n"
            "summary: Root node\n"
            "---\n\n"
            "Links to [[Child]].\n",
            encoding="utf-8",
        )
        (self.vault / "40-Systems" / "Child.md").write_text(
            "---\n"
            "title: Child\n"
            "date: 2026-09-18\n"
            "type: system\n"
            "tags: [system]\n"
            "summary: Child node\n"
            "---\n\n"
            "Links to [[GrandchildService]].\n",
            encoding="utf-8",
        )
        (self.vault / "40-Systems" / "GrandchildService.md").write_text(
            "---\n"
            "title: Grandchild Service\n"
            "date: 2026-09-18\n"
            "type: system\n"
            "tags: [system]\n"
            "summary: Service node\n"
            "ports: [8080]\n"
            "---\n\n"
            "# Grandchild Service\n",
            encoding="utf-8",
        )

        with fts_db_context(self.vault) as con:
            sync_fts_index(self.vault, con)

        _, is_err, data = traverse_graph(self.vault, "Root", depth=3, direction="down")
        self.assertFalse(is_err)
        sinks = [s["name"] for s in data.get("boundary_sinks", [])]
        # GrandchildService should be discovered as a sink even though depth == 2
        self.assertIn("GrandchildService", sinks)

    def test_akatsuki_test_mcp_target_alias(self):
        """Test that akatsuki_test tool accepts 'target' as an alias for 'note'."""
        import akatsuki.storage as storage

        prev = storage.CURRENT_VAULT_OVERRIDE
        try:
            storage.CURRENT_VAULT_OVERRIDE = self.vault
            res, is_err = handle_mcp_call("akatsuki_test", {"target": "Nonexistent", "dry_run": True})
            self.assertFalse(is_err)
            self.assertIn("No machine verification blocks", res)
        finally:
            storage.CURRENT_VAULT_OVERRIDE = prev


if __name__ == "__main__":
    unittest.main()

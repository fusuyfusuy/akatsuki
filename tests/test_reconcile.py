import shutil
import tempfile
import unittest
from pathlib import Path

from akatsuki.core import lint_vault, verify_links, reconcile_vault


class TestReconcileAndClosure(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp()
        self.vault = Path(self.test_dir)
        # Create minimal vault structure
        (self.vault / "01-Daily").mkdir(parents=True)
        (self.vault / "20-Projects").mkdir(parents=True)
        (self.vault / "INDEX.md").write_text(
            "---\ntitle: Index\ndate: 2026-09-14\ntype: moc\ntags: [moc]\nsummary: Index\n---\n# Index\n[[20-Projects/Projects-MOC]]\n",
            encoding="utf-8",
        )
        (self.vault / "20-Projects" / "Projects-MOC.md").write_text(
            "---\ntitle: Projects MOC\ndate: 2026-09-14\ntype: moc\ntags: [moc]\nsummary: Projects\n---\n# Projects MOC\n",
            encoding="utf-8",
        )

    def tearDown(self):
        shutil.rmtree(self.test_dir, ignore_errors=True)

    def test_strict_yaml_lint_failure(self):
        bad_note = self.vault / "20-Projects" / "bad.md"
        bad_note.write_text(
            "---\ntitle: Bad Note\ndate: 2026-09-14\ntype: project\ntags: [test]\nstatus: live\nsummary: Unquoted colon: causes yaml failure\n---\n# Bad Note\n",
            encoding="utf-8",
        )
        out, is_err = lint_vault(self.vault)
        self.assertTrue(is_err)
        self.assertIn("Strict YAML frontmatter syntax error", out)

    def test_graph_closure_unindexed_detection(self):
        unindexed_note = self.vault / "20-Projects" / "orphan_proj.md"
        unindexed_note.write_text(
            "---\ntitle: Orphan Project\ndate: 2026-09-14\ntype: project\ntags: [test]\nstatus: live\nsummary: \"A project\"\n---\n# Orphan\n",
            encoding="utf-8",
        )
        ok, issues = verify_links(self.vault)
        self.assertFalse(ok)
        self.assertTrue(any("Unindexed note" in issue for _, issue in issues))

    def test_reconcile_auto_fixes_and_indexes(self):
        note = self.vault / "20-Projects" / "auto_note.md"
        note.write_text(
            "---\ntitle: Auto Note\ndate: 2026-09-14\ntype: project\ntags: [test]\nstatus: live\nsummary: Unquoted colon: should be quoted\n---\n# Auto Note\n",
            encoding="utf-8",
        )
        msg, is_err = reconcile_vault(self.vault, dry_run=False)
        self.assertFalse(is_err)
        self.assertIn("Auto-quoted frontmatter field 'summary'", msg)
        self.assertIn("Appended unindexed note", msg)

        # Now lint and verify should pass!
        out, lint_err = lint_vault(self.vault)
        self.assertFalse(lint_err, f"Lint failed: {out}")
        ok, issues = verify_links(self.vault)
        self.assertTrue(ok, f"Verify failed: {issues}")


if __name__ == "__main__":
    unittest.main()

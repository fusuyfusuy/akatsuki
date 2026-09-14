import shutil
import tempfile
import unittest
from pathlib import Path

from akatsuki.core import lint_vault, reconcile_vault, verify_links


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
            '---\ntitle: Orphan Project\ndate: 2026-09-14\ntype: project\ntags: [test]\nstatus: live\nsummary: "A project"\n---\n# Orphan\n',
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

    def test_wikilink_with_anchor_resolves_cleanly(self):
        note = self.vault / "20-Projects" / "anchored.md"
        note.write_text(
            '---\ntitle: Anchored\ndate: 2026-09-14\ntype: project\ntags: [test]\nstatus: live\nsummary: "A project with anchors"\n---\n# Anchored\n## Arch\nBody\n',
            encoding="utf-8",
        )
        moc = self.vault / "20-Projects" / "Projects-MOC.md"
        moc.write_text(
            moc.read_text(encoding="utf-8") + "\n- [[anchored#Arch|Arch Link]]\n",
            encoding="utf-8",
        )
        ok, issues = verify_links(self.vault)
        self.assertTrue(ok, f"verify_links failed with anchor: {issues}")

    def test_dump_frontmatter_colon_quoting_and_lint(self):
        from akatsuki.core import dump_frontmatter

        fm = {
            "title": "Colon Test",
            "date": "2026-09-14",
            "type": "project",
            "status": "live",
            "tags": ["test"],
            "summary": "Alert: system status is nominal",
        }
        rendered = dump_frontmatter(fm, "# Colon Test\n\nBody.\n")
        target = self.vault / "20-Projects" / "colon_test.md"
        target.write_text(rendered, encoding="utf-8")

        # Must pass lint without requiring external PyYAML
        out, is_err = lint_vault(self.vault)
        self.assertFalse(is_err, f"Lint failed on dumped frontmatter: {out}")

    def test_nested_relations_parsing(self):
        from akatsuki.core import parse_frontmatter

        raw_yaml = """---
title: Rel Test
relations:
  depends_on:
    - svc_a
    - svc_b
  consumed_by:
    - client_c
---
Body
"""
        meta, _ = parse_frontmatter(raw_yaml)
        self.assertIsInstance(meta.get("relations"), dict)
        self.assertEqual(meta["relations"].get("depends_on"), ["svc_a", "svc_b"])
        self.assertEqual(meta["relations"].get("consumed_by"), ["client_c"])

    def test_blast_radius_exact_matching(self):
        from akatsuki.core import calculate_blast_radius, write_note

        write_note(
            self.vault, "20-Projects/web.md", "---\ntitle: Web\ntype: project\nstatus: live\nsummary: Web\n---\n# Web\n"
        )
        write_note(
            self.vault,
            "20-Projects/webhook.md",
            "---\ntitle: Webhook\ntype: project\nstatus: live\nsummary: Webhook\n---\n# Webhook\n",
        )
        write_note(
            self.vault,
            "20-Projects/consumer.md",
            "---\ntitle: Consumer\ntype: project\nstatus: live\nsummary: Consumer\n---\n# Consumer\n[[webhook]]\n",
        )

        out, is_err = calculate_blast_radius(self.vault, "web")
        self.assertFalse(is_err)
        self.assertIn("None detected in knowledge graph", out)

        out_hook, is_err2 = calculate_blast_radius(self.vault, "webhook")
        self.assertFalse(is_err2)
        self.assertIn("20-Projects/consumer.md", out_hook)


if __name__ == "__main__":
    unittest.main()

import datetime
import os
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from akatsuki.core import (
    append_work_log,
    get_fts_db,
    get_machine_id,
    parse_frontmatter,
    set_note_property,
    sync_fts_index,
    write_note,
)


class TestDeviceTrackingAndFrontmatter(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp()
        self.vault = Path(self.test_dir)
        (self.vault / "01-Daily").mkdir(parents=True, exist_ok=True)
        (self.vault / "20-Projects").mkdir(parents=True, exist_ok=True)

    def tearDown(self):
        shutil.rmtree(self.test_dir, ignore_errors=True)

    def test_device_resolution(self):
        default_id = get_machine_id()
        self.assertTrue(default_id, "default_id must not be empty")

        with patch.dict(os.environ, {"AKATSUKI_HOST": "custom-runner-01"}):
            self.assertEqual(get_machine_id(), "custom-runner-01")

    def test_work_log_device_tagging(self):
        append_work_log(
            self.vault,
            project="my-app",
            summary="initial deploy -> exit 0",
            device="device-alpha",
        )

        now_str = datetime.datetime.now().astimezone().strftime("%Y-%m-%d")
        daily_file = self.vault / "01-Daily" / f"{now_str}.md"
        self.assertTrue(daily_file.exists())

        content = daily_file.read_text(encoding="utf-8")
        self.assertIn("[device-alpha]: [my-app] initial deploy -> exit 0", content)

    def test_frontmatter_stamping_and_sqlite(self):
        with patch.dict(os.environ, {"AKATSUKI_HOST": "node-beta"}):
            note_content = (
                "---\n"
                'title: "Test Project"\n'
                "date: 2026-09-13\n"
                "type: project\n"
                "status: active\n"
                "tags:\n"
                "  - test\n"
                'summary: "A project for testing device tracking"\n'
                "---\n\n"
                "# Test Project\n\nOperational body.\n"
            )
            msg, is_err = write_note(self.vault, "20-Projects/test-project.md", note_content)
            self.assertFalse(is_err, f"write_note failed: {msg}")

            target_note = self.vault / "20-Projects" / "test-project.md"
            fm, _ = parse_frontmatter(target_note.read_text(encoding="utf-8"))
            self.assertEqual(fm.get("updated_by"), "node-beta")
            self.assertTrue(fm.get("updated"))

            # Check daily log entry created by auto-mutation logging
            now_str = datetime.datetime.now().astimezone().strftime("%Y-%m-%d")
            daily_file = self.vault / "01-Daily" / f"{now_str}.md"
            self.assertIn(
                "[node-beta]: [akatsuki] create 20-Projects/test-project.md -> exit 0",
                daily_file.read_text(encoding="utf-8"),
            )

        # Test set_note_property
        with patch.dict(os.environ, {"AKATSUKI_HOST": "node-gamma"}):
            msg, is_err = set_note_property(self.vault, "20-Projects/test-project.md", "status", "maintenance")
            self.assertFalse(is_err, f"set_note_property failed: {msg}")

            fm_updated, _ = parse_frontmatter(target_note.read_text(encoding="utf-8"))
            self.assertEqual(fm_updated.get("status"), "maintenance")
            self.assertEqual(fm_updated.get("updated_by"), "node-gamma")

            # Check SQLite DB
            db = get_fts_db(self.vault)
            sync_fts_index(self.vault, db)
            cur = db.execute("SELECT stem, updated_by, status FROM entities WHERE stem = 'test-project'")
            row = cur.fetchone()
            self.assertIsNotNone(row)
            self.assertEqual(row["updated_by"], "node-gamma")
            self.assertEqual(row["status"], "maintenance")


if __name__ == "__main__":
    unittest.main()

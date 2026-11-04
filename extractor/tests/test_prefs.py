"""What the windows remember (`room_m.prefs`). Stdlib unittest, CPython."""

import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import prefs  # noqa: E402


class Prefs(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.file = os.path.join(self.dir, "RoomMate", "prefs.json")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_it_lives_beside_server_json(self):
        where = prefs.path({"APPDATA": self.dir})
        self.assertEqual(os.path.join(self.dir, "RoomMate", "prefs.json"), where)

    def test_a_section_is_kept_and_read_back(self):
        self.assertTrue(prefs.save_section("export", {"project": "p1", "entities": ["rooms"]}, self.file))
        self.assertEqual({"project": "p1", "entities": ["rooms"]}, prefs.section("export", self.file))

    def test_saving_one_window_keeps_the_others(self):
        prefs.save_section("export", {"project": "p1"}, self.file)
        prefs.save_section("sheets", {"filter": "plan"}, self.file)
        self.assertEqual({"project": "p1"}, prefs.section("export", self.file))
        self.assertEqual({"filter": "plan"}, prefs.section("sheets", self.file))

    def test_nothing_saved_or_a_broken_file_is_an_empty_answer_not_an_error(self):
        self.assertEqual({}, prefs.section("export", self.file))
        os.makedirs(os.path.dirname(self.file))
        with open(self.file, "w") as handle:
            handle.write("{not json")
        self.assertEqual({}, prefs.section("export", self.file))
        self.assertTrue(prefs.save_section("export", {"a": 1}, self.file), "a broken file is replaced")
        self.assertEqual({"a": 1}, prefs.section("export", self.file))

    def test_a_write_that_cannot_happen_is_false_and_never_raised(self):
        blocker = os.path.join(self.dir, "file")
        open(blocker, "w").close()
        self.assertFalse(prefs.save_section("export", {}, os.path.join(blocker, "sub", "prefs.json")))


if __name__ == "__main__":
    unittest.main()

"""Tests for room_m.task_log. Run: python3 extractor/tests/test_task_log.py"""
import os
import shutil
import sys
import tempfile
import unittest

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
sys.path.insert(0, os.path.join(ROOM_M, ".."))

from room_m import task_log  # noqa: E402


class Log(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.target = os.path.join(self.dir, "RoomMate")

    def tearDown(self):
        task_log.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def read(self):
        with open(task_log.path()) as f:
            return f.read()

    def test_before_start_every_call_does_nothing(self):
        task_log.info("a")
        task_log.step("b")
        task_log.exception("c")
        self.assertIsNone(task_log.path())

    def test_the_log_is_in_the_roommate_folder_and_each_line_is_on_disk_as_it_is_written(self):
        path = task_log.start("Tasks", self.target)
        self.assertEqual(os.path.dirname(path), self.target)
        self.assertTrue(path.endswith("RoomMate.log"))
        task_log.step("sheet 50H-01: plan view")
        text = self.read()
        self.assertIn("---- Tasks ----", text)
        self.assertIn("STEP | sheet 50H-01: plan view", text)

    def test_an_exception_is_written_with_its_traceback(self):
        task_log.start("Tasks", self.target)
        try:
            raise ValueError("no title block")
        except ValueError:
            task_log.exception("sheet failed")
        text = self.read()
        self.assertIn("sheet failed", text)
        self.assertIn("ValueError: no title block", text)

    def test_a_second_start_carries_on_in_the_same_file(self):
        first = task_log.start("one", self.target)
        task_log.info("between")
        second = task_log.start("two", self.target)
        self.assertEqual(first, second)
        text = self.read()
        self.assertLess(text.index("---- one ----"), text.index("---- two ----"))

    def test_the_folder_follows_appdata(self):
        self.assertEqual(task_log.log_dir({"APPDATA": "C:\\Users\\x\\AppData\\Roaming"}),
                         os.path.join("C:\\Users\\x\\AppData\\Roaming", "RoomMate"))

    def test_a_folder_that_cannot_be_made_gives_no_log_and_no_error(self):
        blocker = os.path.join(self.dir, "file")
        open(blocker, "w").close()
        self.assertIsNone(task_log.start("Tasks", os.path.join(blocker, "RoomMate")))
        task_log.info("still fine")


if __name__ == "__main__":
    unittest.main()

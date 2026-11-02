"""Tests for room_m.sheet_export_form. Run: python3 extractor/tests/test_sheet_export_form.py"""
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import sheet_export_form as f  # noqa: E402


class SheetExportForm(unittest.TestCase):
    def test_caption(self):
        self.assertEqual(f.rooms_caption(0), "(none)")
        self.assertEqual(f.rooms_caption(1), "1 room")
        self.assertEqual(f.rooms_caption(3), "3 rooms")

    def test_filter_ignores_case_and_blank_lists_everything(self):
        self.assertTrue(f.matches("1.01 Bedroom (L1)", "BED"))
        self.assertTrue(f.matches("1.01 Bedroom (L1)", "  "))
        self.assertFalse(f.matches("1.01 Bedroom (L1)", "bath"))

    def test_problems(self):
        self.assertEqual(len(f.problems(None, 0)), 2)
        self.assertEqual(f.problems({"id": "p"}, 1), [])


if __name__ == "__main__":
    unittest.main()

"""Tests for room_m.blueprint_export_form and the blueprint export window's XAML.
Run: python3 extractor/tests/test_blueprint_export_form.py"""
import os
import re
import sys
import unittest
import xml.etree.ElementTree as ET

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
sys.path.insert(0, os.path.join(ROOM_M, ".."))

from room_m import blueprint_export_form as f  # noqa: E402

X = "{http://schemas.microsoft.com/winfx/2006/xaml}"
NS = "{http://schemas.microsoft.com/winfx/2006/xaml/presentation}"


class Rules(unittest.TestCase):
    def test_captions_and_filter(self):
        self.assertEqual(f.sheets_caption(0), "(none)")
        self.assertEqual(f.sheets_caption(1), "1 sheet")
        self.assertEqual(f.sheets_caption(3), "3 sheets")
        self.assertTrue(f.matches("00.01 RAMPUS", "ramp"))
        self.assertTrue(f.matches("00.01 RAMPUS", " "))
        self.assertFalse(f.matches("00.01 RAMPUS", "bath"))

    def test_the_name_defaults_to_the_room_name_then_its_label(self):
        self.assertEqual(f.default_name({"name": "RAMPUS", "label": "00.01 RAMPUS"}), "RAMPUS")
        self.assertEqual(f.default_name({"name": "", "label": "00.01"}), "00.01")

    def test_a_sheet_ticked_for_a_room_is_taken_for_the_others_only(self):
        rows = [("r1", ["s1", "s2"]), ("r2", ["s3"])]
        self.assertEqual(f.taken_elsewhere(rows, "r1"), {"s3"})
        self.assertEqual(f.taken_elsewhere(rows, "r3"), {"s1", "s2", "s3"})

    def test_what_stops_the_export(self):
        self.assertEqual(len(f.problems(None, None, [])), 3)
        self.assertEqual(f.problems({"id": "p"}, "New", [("A", "Bed", 1)]), [])
        self.assertIn("A: pick at least one room layout sheet.", f.problems({"id": "p"}, "New", [("A", "Bed", 0)]))
        self.assertIn("A: give the blueprint a name.", f.problems({"id": "p"}, "New", [("A", " ", 1)]))

    def test_two_rooms_cannot_share_a_name(self):
        out = f.problems({"id": "p"}, "New", [("A", "Bed", 1), ("B", "bed", 1)])
        self.assertEqual(out, ["B: the name 'bed' is also A's."])

    def test_a_long_list_of_problems_says_how_many_more(self):
        rooms = [(str(i), "", 0) for i in range(5)]
        out = f.problems({"id": "p"}, "New", rooms)
        self.assertEqual(len(out), f.SHOWN + 1)
        self.assertTrue(out[-1].startswith("and "))


class Xaml(unittest.TestCase):
    def setUp(self):
        self.root = ET.parse(os.path.join(ROOM_M, "blueprint_export.xaml")).getroot()
        self.names = [e.get(X + "Name") for e in self.root.iter() if e.get(X + "Name")]

    def test_it_has_no_class_and_no_grid_control(self):
        self.assertIsNone(self.root.get(X + "Class"))
        tags = set(e.tag for e in self.root.iter())
        self.assertNotIn(NS + "DataGrid", tags)
        self.assertNotIn(NS + "ListView", tags)

    def test_the_controls_the_window_reads_exist_once(self):
        for name in ("tx_model", "cb_project", "cb_phase", "tb_find", "sp_rooms", "tx_status", "bt_export"):
            self.assertEqual(1, self.names.count(name), name)

    def test_every_handler_in_the_xaml_is_a_method_of_the_window(self):
        with open(os.path.join(ROOM_M, "blueprint_export_window.py")) as fh:
            source = fh.read()
        handlers = set()
        for e in self.root.iter():
            for attribute in ("Click", "SelectionChanged", "TextChanged"):
                if e.get(attribute):
                    handlers.add(e.get(attribute))
        self.assertTrue(handlers)
        for h in handlers:
            self.assertTrue(re.search(r"def {}\(self, sender, args\)".format(h), source), h)

    def test_the_entry_uses_the_window_and_no_longer_asks_room_by_room(self):
        with open(os.path.join(ROOM_M, "room_mate.py")) as fh:
            text = fh.read()
        self.assertIn("blueprint_export_window.show(", text)
        self.assertNotIn("Blueprint name for room", text)


if __name__ == "__main__":
    unittest.main()

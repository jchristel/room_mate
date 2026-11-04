"""Tests for room_m.sheet_export_form. Run: python3 extractor/tests/test_sheet_export_form.py"""
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import sheet_export_form as f  # noqa: E402


class SheetExportForm(unittest.TestCase):
    def rooms(self, *numbers):
        return [{"number": n, "name": "", "label": n} for n in numbers]

    def test_caption_names_the_room_numbers_not_a_count(self):
        self.assertEqual(f.rooms_caption([]), "(none)")
        self.assertEqual(f.rooms_caption(self.rooms("1.01")), "1.01")
        self.assertEqual(f.rooms_caption(self.rooms("1.01", "1.02", "1.03")), "1.01, 1.02, 1.03")

    def test_a_long_list_is_cut_and_says_how_many_more(self):
        caption = f.rooms_caption(self.rooms(*["10{}".format(i) for i in range(30)]))
        self.assertIn("more", caption)
        self.assertLess(len(caption), f.CAPTION_LIMIT + 12)
        self.assertTrue(caption.startswith("100, 101"))

    def test_a_room_with_no_number_shows_its_label(self):
        self.assertEqual("Store (L1)", f.rooms_caption([{"number": "", "label": "Store (L1)"}]))

    def test_sheet_filter_takes_words_all_of_which_must_match_and_minus_words_that_must_not(self):
        label = "A101 - Ground Floor Roof Plan"
        self.assertTrue(f.filter_matches(label, ""))
        self.assertTrue(f.filter_matches(label, "a101 plan"))
        self.assertFalse(f.filter_matches(label, "a101 section"))
        self.assertFalse(f.filter_matches(label, "plan -roof"))
        self.assertTrue(f.filter_matches(label, "plan -ceiling"))

    def test_a_lone_minus_is_just_a_word(self):
        self.assertTrue(f.filter_matches("A - B", "-"))

    def sheet(self, number, name=""):
        return {"number": number, "name": name}

    def room(self, number, name=""):
        return {"number": number, "name": name}

    def test_the_example_rule_a_room_contains_the_sheet_number(self):
        rule = f.make_rule("number", "inside", "number")
        rooms = [self.room("101-A"), self.room("102"), self.room("")]
        self.assertEqual(["101-A"], [r["number"] for r in f.rooms_for_sheet(rule, self.sheet("101"), rooms)])

    def test_the_other_direction_and_equals(self):
        rooms = [self.room("101"), self.room("102")]
        contains = f.make_rule("number", "contains", "number")
        self.assertEqual(["101"], [r["number"] for r in f.rooms_for_sheet(contains, self.sheet("A-101"), rooms)])
        equals = f.make_rule("number", "equals", "number")
        self.assertEqual(["102"], [r["number"] for r in f.rooms_for_sheet(equals, self.sheet("102"), rooms)])

    def test_either_compares_number_and_name_and_ignores_case(self):
        rule = f.make_rule("either", "inside", "either")
        rooms = [self.room("7", "Kitchen")]
        self.assertTrue(f.rule_matches(rule, self.sheet("X", "kitchen"), rooms[0]))
        self.assertFalse(f.rule_matches(rule, self.sheet("X", "bath"), rooms[0]))

    def test_a_blank_value_never_matches(self):
        rule = f.make_rule("number", "inside", "number")
        self.assertFalse(f.rule_matches(rule, self.sheet(""), self.room("101")))
        self.assertFalse(f.rule_matches(rule, self.sheet("101"), self.room("")))

    def test_no_rule_matches_nothing_and_an_unknown_part_is_no_rule(self):
        self.assertEqual([], f.rooms_for_sheet(None, self.sheet("1"), [self.room("1")]))
        self.assertIsNone(f.make_rule("colour", "inside", "number"))
        self.assertEqual("No rule: choose the rooms by hand.", f.describe_rule(None))

    def test_a_rule_reads_as_a_sentence(self):
        self.assertEqual("Sheet number is part of room number.", f.describe_rule(f.make_rule("number", "inside", "number")))

    def test_filter_ignores_case_and_blank_lists_everything(self):
        self.assertTrue(f.matches("1.01 Bedroom (L1)", "BED"))
        self.assertTrue(f.matches("1.01 Bedroom (L1)", "  "))
        self.assertFalse(f.matches("1.01 Bedroom (L1)", "bath"))

    def test_problems(self):
        self.assertEqual(len(f.problems(None, 0)), 2)
        self.assertEqual(f.problems({"id": "p"}, 1), [])


class SheetXaml(unittest.TestCase):
    """The window is drawn from XAML that names its handlers as plain methods; a handler that is missing only
    fails when Revit opens the window, so each is checked here."""

    def setUp(self):
        import re
        import xml.etree.ElementTree as ET
        room_m = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
        self.root = ET.parse(os.path.join(room_m, "sheet_export.xaml")).getroot()
        with open(os.path.join(room_m, "sheet_export_window.py")) as handle:
            self.source = handle.read()
        self.methods = set(re.findall(r"def (\w+)\(self", self.source))

    def test_every_handler_in_the_xaml_is_a_method_of_the_window(self):
        handlers = set()
        for e in self.root.iter():
            for attribute in ("Click", "TextChanged", "SelectionChanged", "Checked", "Unchecked"):
                if e.get(attribute):
                    handlers.add(e.get(attribute))
        self.assertTrue(handlers)
        for h in handlers:
            self.assertIn(h, self.methods, h)

    def test_every_control_the_window_reads_is_in_the_xaml(self):
        import re
        X = "{http://schemas.microsoft.com/winfx/2006/xaml}"
        named = set(e.get(X + "Name") for e in self.root.iter() if e.get(X + "Name"))
        used = set(re.findall(r"self\.((?:tx|tb|cb|sp|bt)_\w+)", self.source))
        self.assertEqual(set(), used - named)

    def test_it_has_no_class_and_no_grid_control(self):
        X = "{http://schemas.microsoft.com/winfx/2006/xaml}"
        self.assertIsNone(self.root.get(X + "Class"))
        tags = set(e.tag.split("}")[1] for e in self.root.iter())
        self.assertFalse(tags & {"DataGrid", "ListView"})


class Remembered(unittest.TestCase):
    projects = [{"id": "a"}, {"id": "b"}]

    def test_project_filter_and_rule_come_back(self):
        rule = f.make_rule("number", "inside", "number")
        index, text, got = f.remembered(f.to_save(self.projects[1], "plan -roof", rule), self.projects)
        self.assertEqual((1, "plan -roof", rule), (index, text, got))

    def test_a_vanished_project_and_a_bad_rule_are_dropped(self):
        index, text, rule = f.remembered({"project": "gone", "filter": "x", "rule": {"sheet": "colour"}}, self.projects)
        self.assertEqual((None, "x", None), (index, text, rule))

    def test_nothing_saved(self):
        self.assertEqual((None, "", None), f.remembered({}, self.projects))


if __name__ == "__main__":
    unittest.main()

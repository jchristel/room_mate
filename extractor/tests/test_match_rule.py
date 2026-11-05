"""Tests for room_m.match_rule. Run: python3 extractor/tests/test_match_rule.py"""
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import match_rule as m  # noqa: E402


def sheet(number, name="", **more):
    params = {"Sheet Number": number, "Sheet Name": name}
    params.update(more)
    return {"id": "s" + number, "number": number, "params": params}


def room(number, name="", **more):
    params = {"Number": number, "Name": name}
    params.update(more)
    return {"id": "r" + number, "number": number, "params": params}


class Fill(unittest.TestCase):
    def test_placeholders_are_filled_without_regard_to_case(self):
        self.assertEqual("101-kitchen", m.fill("{number}-{NAME}", room("101", "Kitchen")["params"]))

    def test_a_missing_or_blank_parameter_means_the_side_is_empty(self):
        self.assertIsNone(m.fill("{Number}", {"Number": "  "}))
        self.assertIsNone(m.fill("{Colour}", {"Number": "1"}))
        self.assertIsNone(m.fill("{Number}-{Colour}", {"Number": "1"}))

    def test_text_without_placeholders_is_a_constant(self):
        self.assertEqual("ward", m.fill("Ward", {}))


class Rule(unittest.TestCase):
    def test_the_example_the_sheet_number_is_part_of_the_room_number(self):
        rule = m.make_rule("{Sheet Number}", "inside", "{Number}")
        rooms = [room("101-A"), room("102"), room("")]
        self.assertEqual(["101-A"], [r["number"] for r in m.rooms_for_sheet(rule, sheet("101"), rooms)])

    def test_any_sheet_parameter_against_any_room_parameter(self):
        rule = m.make_rule("{Discipline}", "equals", "{Department}")
        self.assertTrue(m.pair_matches(rule, sheet("1", Discipline="Nursing"), room("7", Department="nursing")))
        self.assertFalse(m.pair_matches(rule, sheet("1", Discipline="Nursing"), room("7", Department="Imaging")))

    def test_contains_and_a_mixed_template(self):
        contains = m.make_rule("{Sheet Number}", "contains", "{Number}")
        self.assertTrue(m.pair_matches(contains, sheet("A-101"), room("101")))
        mixed = m.make_rule("{Sheet Name}", "equals", "RLS {Number} {Name}")
        self.assertTrue(m.pair_matches(mixed, sheet("9", "RLS 101 Bed"), room("101", "bed")))

    def test_a_blank_side_never_matches(self):
        rule = m.make_rule("{Sheet Number}", "inside", "{Number}")
        self.assertFalse(m.pair_matches(rule, sheet(""), room("101")))
        self.assertFalse(m.pair_matches(rule, sheet("101"), room("")))

    def test_no_rule_matches_nothing_and_a_bad_one_is_no_rule(self):
        self.assertFalse(m.pair_matches(None, sheet("1"), room("1")))
        self.assertIsNone(m.make_rule("", "inside", "{Number}"))
        self.assertIsNone(m.make_rule("{A}", "colour", "{B}"))

    def test_sheets_for_a_room_skip_the_ones_another_room_holds(self):
        rule = m.make_rule("{Sheet Number}", "contains", "{Number}")
        sheets = [sheet("101"), sheet("101-B")]
        got = m.sheets_for_room(rule, room("101"), sheets, taken={"s101"})
        self.assertEqual(["101-B"], [s["number"] for s in got])

    def test_a_placeholder_nobody_has_is_said(self):
        rule = m.make_rule("{Sheet Numbr}", "inside", "{Number}")
        said = m.unknown_parameters(rule, [sheet("1")], [room("1")])
        self.assertEqual(["No sheet has a parameter called 'Sheet Numbr'."], said)
        self.assertEqual([], m.unknown_parameters(rule, [], []))


class Saved(unittest.TestCase):
    def test_a_saved_rule_round_trips(self):
        rule = m.make_rule("{A}", "equals", "{B}")
        self.assertEqual(rule, m.rule_from_saved(rule))

    def test_garbage_is_no_rule(self):
        for bad in (None, "x", {}, {"sheet": 3, "relation": "equals", "room": "{B}"}):
            self.assertIsNone(m.rule_from_saved(bad))


class Caption(unittest.TestCase):
    def test_numbers_then_more(self):
        entries = [{"number": "10{}".format(i)} for i in range(30)]
        self.assertTrue(m.numbers_caption(entries).startswith("100, 101"))
        self.assertIn("more", m.numbers_caption(entries))
        self.assertEqual("(none)", m.numbers_caption([]))


class Duplicates(unittest.TestCase):
    def test_blanks_are_not_duplicates(self):
        self.assertEqual({0, 2}, m.duplicates(["Bed", "", " bed", ""]))


if __name__ == "__main__":
    unittest.main()

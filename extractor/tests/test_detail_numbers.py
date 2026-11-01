"""Detail numbering of RLS viewports (`room_m.detail_numbers`). Stdlib unittest, CPython."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import detail_numbers as dn  # noqa: E402

COUNTS = {"elevation": 4, "plan": 1, "three_d": 1}


class Labels(unittest.TestCase):
    def test_numeric_and_alphabetic(self):
        self.assertEqual(["1", "2", "10"], [dn.label("numeric", n) for n in (1, 2, 10)])
        self.assertEqual(["A", "Z", "AA", "AB", "AZ", "BA"], [dn.label("alphabetic", n) for n in (1, 26, 27, 28, 52, 53)])


class Assign(unittest.TestCase):
    def test_no_setting_is_numeric_and_one_run_in_the_order_elevations_plan_3d(self):
        out = dn.assign(COUNTS, {})
        self.assertEqual(["1", "2", "3", "4"], out["elevation"])
        self.assertEqual(["5"], out["plan"])
        self.assertEqual(["6"], out["three_d"])

    def test_all_alphabetic_is_one_run_too(self):
        s = dict((k, "alphabetic") for k in dn.SETTING_OF.values())
        out = dn.assign(COUNTS, s)
        self.assertEqual(["A", "B", "C", "D"], out["elevation"])
        self.assertEqual(["E"], out["plan"])
        self.assertEqual(["F"], out["three_d"])

    def test_a_kind_with_its_own_style_starts_again(self):
        out = dn.assign(COUNTS, {"plan_detail_numbering": "alphabetic"})
        self.assertEqual(["1", "2", "3", "4"], out["elevation"])
        self.assertEqual(["A"], out["plan"])
        self.assertEqual(["5"], out["three_d"], "3D shares the numeric run with the elevations")

    def test_an_unknown_style_is_numeric(self):
        self.assertEqual("numeric", dn.style_of({"plan_detail_numbering": "roman"}, "plan"))

    def test_placeholders_are_distinct(self):
        p = dn.placeholders(3)
        self.assertEqual(["place holder 1", "place holder 2", "place holder 3"], p)

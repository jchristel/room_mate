"""A 3D view's section box as heights above its room's level (`room_m.section_box`). Stdlib unittest, CPython."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import section_box as sb  # noqa: E402


def three_d(low, high, active=True):
    return {"vp_type": "three d", "view": {
        "section_box_active": active,
        "section_box_min": {"x": 0, "y": 0, "z": low},
        "section_box_max": {"x": 9, "y": 9, "z": high}}}


class AboveFloor(unittest.TestCase):
    def test_heights_are_measured_from_the_rooms_level(self):
        # level at 450 mm, box from 150 to 3450: 300 below the level to 3000 above it
        self.assertEqual({"min": -300.0, "max": 3000.0}, sb.above_floor(three_d(150, 3450)["view"], 450.0))

    def test_a_box_that_is_off_or_missing_or_empty_gives_none(self):
        self.assertIsNone(sb.above_floor(three_d(0, 10, active=False)["view"], 0.0))
        self.assertIsNone(sb.above_floor({"section_box_active": True}, 0.0))
        self.assertIsNone(sb.above_floor(three_d(10, 10)["view"], 0.0))
        self.assertIsNone(sb.above_floor(None, 0.0))


class Attach(unittest.TestCase):
    def test_only_3d_viewports_with_a_box_get_the_heights(self):
        sheet = {"view_ports": [three_d(450, 3000), {"vp_type": "floor plan", "view": {}}, three_d(0, 5, active=False)]}
        self.assertEqual(1, sb.attach(sheet, 450.0))
        self.assertEqual({"min": 0.0, "max": 2550.0}, sheet["view_ports"][0]["view"][sb.KEY])
        self.assertNotIn(sb.KEY, sheet["view_ports"][1]["view"])
        self.assertNotIn(sb.KEY, sheet["view_ports"][2]["view"])


class Apply(unittest.TestCase):
    def test_a_viewport_with_heights_gives_them_in_feet_and_one_without_gives_none(self):
        vp = {"view": {sb.KEY: {"min": 304.8, "max": 609.6}}}
        lo, hi = sb.above_floor_ft(vp, lambda mm: mm / 304.8)
        self.assertAlmostEqual(1.0, lo)
        self.assertAlmostEqual(2.0, hi)
        self.assertIsNone(sb.above_floor_ft({"view": {}}, lambda mm: mm))
        self.assertIsNone(sb.above_floor_ft(None, lambda mm: mm))


if __name__ == "__main__":
    unittest.main()

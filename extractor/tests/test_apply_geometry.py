"""The arithmetic of `room_m.apply_geometry`. Stdlib unittest, CPython."""

import math
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import apply_geometry as g  # noqa: E402


def item(x=1000.0, y=2000.0, fx=1.0, fy=0.0, family="Bed", type_name="Single"):
    return {"x_mm": x, "y_mm": y, "facing_x": fx, "facing_y": fy, "family_name": family, "type_name": type_name}


def rule(head=(10.0, 4.0), rotation=0.0, leader=True, vp="floor plan", family="Bed", type_name="Single"):
    return {
        "key": {"tagged_family": family, "tagged_type": type_name, "viewport_type": vp},
        "placement": {"head": {"x": head[0], "y": head[1]}, "rotation": rotation, "has_leader": leader},
    }


class TagPlacement(unittest.TestCase):
    def test_paper_millimetres_become_model_millimetres_at_the_scale(self):
        p = g.tag_placement(item(), rule(head=(20.0, 0.0)), 50)
        self.assertAlmostEqual(p["head"][0], 1000.0 + 1000.0)
        self.assertAlmostEqual(p["head"][1], 2000.0)

    def test_the_offset_turns_with_the_facing(self):
        # facing +Y: x along the facing is +Y, y to the left is -X
        p = g.tag_placement(item(fx=0.0, fy=1.0), rule(head=(10.0, 4.0)), 100)
        self.assertAlmostEqual(p["head"][0], 1000.0 - 400.0)
        self.assertAlmostEqual(p["head"][1], 2000.0 + 1000.0)

    def test_rotation_is_relative_to_the_facing(self):
        p = g.tag_placement(item(fx=0.0, fy=1.0), rule(rotation=0.5), 50)
        self.assertAlmostEqual(p["rotation"], math.pi / 2 + 0.5)

    def test_no_scale_gives_no_placement(self):
        self.assertIsNone(g.tag_placement(item(), rule(), None))
        self.assertIsNone(g.tag_placement(item(), rule(), 0))

    def test_elbow_and_end_follow_the_head(self):
        r = rule()
        r["placement"]["elbow"] = {"x": 5.0, "y": 0.0}
        r["placement"]["end"] = {"x": 1.0, "y": 1.0}
        p = g.tag_placement(item(), r, 10)
        self.assertAlmostEqual(p["elbow"][0], 1050.0)
        self.assertAlmostEqual(p["end"][1], 2010.0)

    def test_an_item_with_no_facing_is_not_rotated(self):
        self.assertEqual(g.facing_angle(item(fx=0.0, fy=0.0)), 0.0)


class Rules(unittest.TestCase):
    def test_rules_match_by_names_and_viewport_type_ignoring_case(self):
        rules = [rule(), rule(vp="elevation"), rule(family="Chair")]
        got = g.rules_for_item(rules, item(family="BED", type_name="single "), "Floor Plan")
        self.assertEqual(len(got), 1)


class Numbers(unittest.TestCase):
    def test_default_number(self):
        self.assertEqual(g.rls_number("01.02"), "RLS-01.02")

    def test_several_sheets_are_counted(self):
        self.assertEqual(g.rls_number("01.02", 1, 3), "RLS-01.02-2")

    def test_a_template_with_n_places_it(self):
        self.assertEqual(g.rls_number("5", 0, 2, "{room}/{n}"), "5-1")

    def test_characters_revit_refuses_are_replaced(self):
        self.assertEqual(g.rls_number("A:B"), "RLS-A-B")


class Titleblocks(unittest.TestCase):
    tbs = [
        {"family": "TB A1", "type": "Standard"},
        {"family": "TB", "type": "A3 landscape"},
        {"family": "TB A10", "type": "Standard"},
    ]

    def test_the_size_must_be_a_whole_token(self):
        got = g.titleblock_candidates("A1", self.tbs)
        self.assertEqual([t["family"] for t in got], ["TB A1"])

    def test_found_in_the_type_name(self):
        self.assertEqual(len(g.titleblock_candidates("A3", self.tbs)), 1)

    def test_none_for_a_size_nothing_has(self):
        self.assertEqual(g.titleblock_candidates("A0", self.tbs), [])

    def test_no_size_accepts_any(self):
        self.assertEqual(len(g.titleblock_candidates(None, self.tbs)), 3)


class Viewports(unittest.TestCase):
    def test_centre_is_converted_from_mm(self):
        c = g.viewport_centre_ft({"centre_point": {"x": 304.8, "y": 609.6}})
        self.assertAlmostEqual(c[0], 1.0)
        self.assertAlmostEqual(c[1], 2.0)

    def test_crop_never_shrinks_below_the_room(self):
        vp = {"view": {"bounding_box": {"min": {"x": 0, "y": 0}, "max": {"x": 3000, "y": 2000}}}}
        self.assertEqual(g.crop_size_mm(vp, 4000, 1000), (4000, 2000))

    def test_crop_reads_flat_fields_too(self):
        vp = {"view": {"bounding_box": {"min_x": 0, "min_y": 0, "max_x": 10, "max_y": 20}}}
        self.assertEqual(g.crop_size_mm(vp), (10, 20))

    def test_no_crop_exported(self):
        self.assertIsNone(g.crop_size_mm({"view": {}}))

    def test_scale(self):
        self.assertEqual(g.viewport_scale({"view": {"view_scale": 50}}), 50)
        self.assertIsNone(g.viewport_scale({"view": {"view_scale": 0}}))


if __name__ == "__main__":
    unittest.main()

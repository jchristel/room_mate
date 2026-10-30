"""Tests for room_m.room_figures (pure half). Run: python3 extractor/tests/test_room_figures.py"""
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import room_figures as f  # noqa: E402


def rect(x0, y0, x1, y1):
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]


class RoomFigures(unittest.TestCase):
    def test_a_rectangle_gives_long_then_short(self):
        self.assertEqual(f.min_rectangle(rect(0, 0, 3000, 4000)), (4000.0, 3000.0))

    def test_a_turned_rectangle_reads_the_same(self):
        # a 5000 x 1250 strip turned by atan(3/4)
        pts = [(0, 0), (4000, 3000), (4000 - 750, 3000 + 1000), (-750, 1000)]
        long_side, short_side = f.min_rectangle(pts)
        self.assertAlmostEqual(long_side, 5000.0, places=3)
        self.assertAlmostEqual(short_side, 1250.0, places=3)

    def test_too_few_points_give_none(self):
        self.assertIsNone(f.min_rectangle([(0, 0), (1, 1)]))

    def test_one_room_is_its_own_figure(self):
        got = f.combine([{"outline": rect(0, 0, 4000, 3000), "items": 6}])
        self.assertEqual(got, {"x_mm": 4000.0, "y_mm": 3000.0, "area_m2": 12.0, "items": 6})

    def test_two_rooms_share_one_rectangle_and_sum_area_and_items(self):
        got = f.combine([
            {"outline": rect(0, 0, 4000, 3000), "items": 6},
            {"outline": rect(4000, 0, 6000, 3000), "items": 2},
        ])
        self.assertEqual(got["x_mm"], 6000.0)
        self.assertEqual(got["y_mm"], 3000.0)
        self.assertEqual(got["area_m2"], 18.0)
        self.assertEqual(got["items"], 8)

    def test_no_outline_gives_none(self):
        self.assertIsNone(f.combine([{"outline": [], "items": 3}]))

    def test_label(self):
        self.assertEqual(f.room_label({"number": "1.01", "name": "Bedroom"}), "1.01 Bedroom")


if __name__ == "__main__":
    unittest.main()

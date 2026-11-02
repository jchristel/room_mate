"""Walls between a 3D camera and the room (`room_m.view_walls`). Stdlib unittest, CPython."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import view_walls as vw  # noqa: E402

# a 4 x 3 room with its walls named by side
ROOM = [
    ("south", (0, 0), (4, 0)),
    ("east", (4, 0), (4, 3)),
    ("north", (4, 3), (0, 3)),
    ("west", (0, 3), (0, 0)),
]
CENTRE = (2, 1.5)


class Crossing(unittest.TestCase):
    def test_segments_that_cross_or_touch_meet_and_apart_ones_do_not(self):
        self.assertTrue(vw.segments_cross((0, 0), (2, 2), (0, 2), (2, 0)))
        self.assertTrue(vw.segments_cross((0, 0), (2, 0), (2, 0), (2, 5)))  # an end on the other
        self.assertFalse(vw.segments_cross((0, 0), (1, 0), (2, -1), (2, 1)))  # on its line, short of it
        self.assertFalse(vw.segments_cross((0, 0), (1, 1), (5, 5), (6, 5)))


class Eye(unittest.TestCase):
    def test_the_camera_is_back_along_the_direction(self):
        e = vw.eye_xy(CENTRE, (0, 5, -5), 10.0)
        self.assertAlmostEqual(2.0, e[0])
        self.assertAlmostEqual(-8.5, e[1])

    def test_a_view_straight_down_has_no_plan_eye(self):
        self.assertIsNone(vw.eye_xy(CENTRE, (0, 0, -1), 10.0))


class Blocking(unittest.TestCase):
    def test_looking_north_the_south_wall_is_in_the_way(self):
        eye = vw.eye_xy(CENTRE, (0, 1, -1), 20.0)
        self.assertEqual(["south"], vw.blocking(eye, CENTRE, ROOM))

    def test_looking_along_a_diagonal_both_walls_of_the_near_corner_go(self):
        eye = vw.eye_xy((2, 1.5), (1, 1, -1), 20.0)  # from the south west
        # the line runs along the diagonal of a 2:1.5 box, so it leaves through the south wall
        self.assertIn(vw.blocking(eye, CENTRE, ROOM)[0], ("south", "west"))
        corner = vw.blocking((-5, -5), (2, 2), [("south", (0, 0), (4, 0)), ("west", (0, 4), (0, 0))])
        self.assertEqual(["south", "west"], corner)

    def test_a_wall_with_two_pieces_is_named_once(self):
        pieces = [("w", (0, 0), (2, 0)), ("w", (2, 0), (4, 0))]
        self.assertEqual(["w"], vw.blocking((2, -5), (2, 3), pieces))

    def test_no_eye_gives_none(self):
        self.assertEqual([], vw.blocking(None, CENTRE, ROOM))


if __name__ == "__main__":
    unittest.main()

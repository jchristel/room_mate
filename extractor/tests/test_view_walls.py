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


class WallsToHide(unittest.TestCase):
    """The sketch in `docs/3D wall hiding logic.pdf`: a camera off the south west corner of a room whose line to
    the middle crosses only the west wall still has the south wall across the near edge of its view."""

    def test_from_a_corner_both_walls_of_the_corner_go_though_the_line_crosses_one(self):
        eye, centre = (-6.0, -2.0), (2.0, 1.5)
        self.assertEqual(["west"], vw.blocking(eye, centre, ROOM), "the line alone finds one")
        self.assertEqual(["south", "west"], vw.walls_to_hide(eye, centre, [ROOM]))

    def test_square_on_only_the_near_wall_goes(self):
        eye = vw.eye_xy(CENTRE, (0, 1, -1), 20.0)
        self.assertEqual(["south"], vw.walls_to_hide(eye, CENTRE, [ROOM]))

    def test_the_loop_may_run_either_way(self):
        clockwise = [(k, b, a) for k, a, b in reversed(ROOM)]
        self.assertEqual(["south", "west"], sorted(vw.walls_to_hide((-6.0, -2.0), CENTRE, [clockwise])))

    def test_a_wall_the_camera_looks_along_stays(self):
        self.assertFalse(vw.faces((-10.0, 0.01), (0, 0), (4, 0), True))
        self.assertTrue(vw.faces((2.0, -10.0), (0, 0), (4, 0), True))

    def test_a_piece_that_is_not_a_wall_is_never_hidden_but_keeps_the_loop_direction(self):
        loop = [(None, (0, 0), (4, 0)), ("east", (4, 0), (4, 3)), ("north", (4, 3), (0, 3)), ("west", (0, 3), (0, 0))]
        self.assertEqual(["west"], vw.walls_to_hide((-6.0, -2.0), CENTRE, [loop]))

    def test_a_hole_wall_goes_only_when_the_line_crosses_it(self):
        column = [("c_north", (1.0, 1.0), (1.2, 1.0)), ("c_east", (1.2, 1.0), (1.2, 0.8)),
                  ("c_south", (1.2, 0.8), (1.0, 0.8)), ("c_west", (1.0, 0.8), (1.0, 1.0))]
        hidden = vw.walls_to_hide((-6.0, -2.0), CENTRE, [ROOM, column])
        self.assertEqual(["south", "west"], hidden)

    def test_no_eye_gives_none(self):
        self.assertEqual([], vw.walls_to_hide(None, CENTRE, [ROOM]))


class SeeThrough(unittest.TestCase):
    """The task's items along the walls (`rls.wall_items`), kept where the wall beside them is hidden. Millimetres."""

    PIECES = [(1, (0, 0), (4000, 0)), (2, (4000, 0), (4000, 3000)), (3, (4000, 3000), (0, 3000)), (4, (0, 3000), (0, 0))]
    LISTED = {"band_mm": 600.0, "items": [
        {"id": "wardrobe", "footprint": [[500, 50], [1700, 50], [1700, 650], [500, 650]]},
        {"id": "desk", "footprint": [[3500, 1000], [3950, 1000], [3950, 2000], [3500, 2000]]},
        {"id": "rail", "footprint": [], "host_wall": "4"},
    ]}

    def test_an_item_goes_with_the_wall_it_stands_against_or_is_hosted_by(self):
        self.assertEqual(["wardrobe", "rail"], vw.items_to_see_through(self.LISTED, [1, 4], self.PIECES))
        self.assertEqual(["desk"], vw.items_to_see_through(self.LISTED, [2], self.PIECES))

    def test_nothing_hidden_or_nothing_listed_sees_through_nothing(self):
        self.assertEqual([], vw.items_to_see_through(self.LISTED, [], self.PIECES))
        self.assertEqual([], vw.items_to_see_through(None, [1], self.PIECES))


if __name__ == "__main__":
    unittest.main()

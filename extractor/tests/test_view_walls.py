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
        self.assertAlmostEqual(1.5 - 10.0 / 2 ** 0.5, e[1], msg="10 back along the unit direction, at 45 degrees down")
        self.assertAlmostEqual(-8.5, vw.eye_xy(CENTRE, (0, 5, 0), 10.0)[1], msg="level: all of it in plan")

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


class WallsAlong(unittest.TestCase):
    """The server's hidden edges (`rls.three_ds[n].hide`, mm) turned into the walls with a face along them."""

    SOUTH = [[0, 0], [4000, 0]]
    WALLS = [
        ("south", (-100, -100), (4100, -100), 100.0),   # 200 thick, inner face on the edge
        ("thick", (0, -240), (4000, -240), 225.0),      # 450 thick: its centre is past the band, its face is not
        ("west", (-100, -100), (-100, 3100), 100.0),    # the side wall: meets the south edge end on
        ("lining", (500, 30), (1500, 30), 25.0),        # a lining in front of the south wall
        ("far", (0, -900), (4000, -900), 100.0),        # the next room's wall beyond
    ]

    def test_the_walls_with_a_face_along_an_edge_go_and_the_side_wall_stays(self):
        self.assertEqual(["south", "thick", "lining"], vw.walls_along(self.WALLS, [self.SOUTH], 200.0))

    def test_no_edges_hide_nothing(self):
        self.assertEqual([], vw.walls_along(self.WALLS, [], 200.0))

    def test_the_faces_lie_either_side_of_the_location_line(self):
        faces = vw.faces_of((0, 0), (10, 0), 2.0)
        self.assertEqual([((0, 2.0), (10, 2.0)), ((0, -2.0), (10, -2.0))], [tuple(map(tuple, f)) for f in faces])


if __name__ == "__main__":
    unittest.main()

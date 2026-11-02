"""`room_m.ring_tidy`: collinear vertices go, corners and spikes stay. Stdlib unittest, CPython."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m.ring_tidy import without_collinear  # noqa: E402


class WithoutCollinear(unittest.TestCase):
    def test_two_walls_on_one_line_are_one_segment(self):
        ring = [(0, 0), (5, 0), (10, 0), (10, 8), (0, 8)]
        self.assertEqual(without_collinear(ring), [(0, 0), (10, 0), (10, 8), (0, 8)])

    def test_a_vertex_a_hair_off_the_line_goes_and_a_real_jog_stays(self):
        hair = 0.5 / 304.8 / 2
        self.assertEqual(len(without_collinear([(0, 0), (5, hair), (10, 0), (10, 8), (0, 8)])), 4)
        jog = [(0, 0), (5, 0.2), (10, 0), (10, 8), (0, 8)]  # 61 mm out
        self.assertEqual(len(without_collinear(jog)), 5)

    def test_a_spike_that_doubles_back_stays(self):
        ring = [(0, 0), (10, 0), (12, 0), (10, 0.0001), (10, 8), (0, 8)]
        self.assertIn((12.0, 0.0), without_collinear(ring))

    def test_a_run_of_collinear_vertices_all_go(self):
        ring = [(0, 0), (2, 0), (4, 0), (6, 0), (10, 0), (10, 8), (0, 8)]
        self.assertEqual(without_collinear(ring), [(0, 0), (10, 0), (10, 8), (0, 8)])

    def test_never_fewer_than_three(self):
        self.assertEqual(len(without_collinear([(0, 0), (5, 0), (10, 0)])), 3)

    def test_the_first_vertex_can_go_too(self):
        ring = [(5, 0), (10, 0), (10, 8), (0, 8), (0, 0)]
        self.assertEqual(len(without_collinear(ring)), 4)


if __name__ == "__main__":
    unittest.main()

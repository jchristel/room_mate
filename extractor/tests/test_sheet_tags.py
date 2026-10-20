"""`room_m.sheet_tags`: tags leave the sheet and come back as observations. CPython."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import sheet_tags as st  # noqa: E402


def a_sheet():
    return {
        "view_ports": [
            {"vp_type": "floor plan", "view": {"id": 7, "view_scale": 50, "tags": [{"id": 1}, {"id": 2}]}},
            {"vp_type": "elevation", "tags": ["old"], "view": {"id": 8, "view_scale": 20}},
            {"vp_type": "schedule", "view": {"id": 9}},
        ],
        "tags": ["old"],
    }


class TakeTags(unittest.TestCase):
    def test_the_sheet_is_left_without_tags(self):
        sheet = a_sheet()
        st.take_tags(sheet)
        self.assertNotIn("tags", sheet)
        for port in sheet["view_ports"]:
            self.assertNotIn("tags", port)
            self.assertNotIn("tags", port["view"])

    def test_each_tag_becomes_an_observation_with_its_view(self):
        found = st.take_tags(a_sheet(), {"sheet_number": "RLS-1"})
        self.assertEqual([1, 2], [o["tag"]["id"] for o in found])
        self.assertEqual(("floor plan", 7, 50, "RLS-1"),
                         (found[0]["viewport_type"], found[0]["view_id"], found[0]["view_scale"], found[0]["sheet"]["sheet_number"]))

    def test_a_sheet_with_no_tags_or_no_views_is_fine(self):
        self.assertEqual([], st.take_tags({"view_ports": [{"view": {}}]}))
        self.assertEqual([], st.take_tags({}))
        self.assertEqual([], st.take_tags(None))

    def test_problems_are_described_and_nothing_when_there_are_none(self):
        self.assertEqual([], st.describe_problems("RLS-1", []))
        lines = st.describe_problems("RLS-1", ["tag 5 in view 7: boom"])
        self.assertIn("1 tag(s)", lines[0])
        self.assertIn("boom", lines[1])


class Outcome(unittest.TestCase):
    def test_the_server_answer_is_described_with_the_first_reasons(self):
        skipped = [{"tag_id": i, "reason": "no facing"} for i in range(7)]
        lines = st.describe_outcome({"samples": 3, "rules_changed": 2, "skipped": skipped})
        self.assertEqual("3 tag(s) taken into 2 tag rule(s)", lines[0])
        self.assertEqual("7 tag(s) not used", lines[1])
        self.assertEqual("    and 2 more", lines[-1])

    def test_a_clean_answer_is_one_line(self):
        self.assertEqual(1, len(st.describe_outcome({"samples": 1, "rules_changed": 1})))


if __name__ == "__main__":
    unittest.main()

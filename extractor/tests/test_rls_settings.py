"""The settings hand-off (`room_m.rls_settings`, `room_m.project_settings` sections), pure half. CPython."""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import project_settings as ps  # noqa: E402
from room_m import rls_settings as rs  # noqa: E402

COMPLETE = {
    "sheet_sizes": {"a0": "T : A0", "a1": "T : A1", "a2": "T : A2", "a3": "T : A3"},
    "plan_view_type": "p", "elevation_view_type": "e", "schedule_base": "s",
    "sheet_number": "RLS-{Number}", "sheet_name": "{Name}", "view_name": "{Name}",
}


class Missing(unittest.TestCase):
    def test_nothing_chosen_lists_everything_that_stops_it(self):
        todo = rs.missing({})
        self.assertIn("title block for A0", todo)
        self.assertIn("schedule base", todo)
        self.assertEqual(4 + 6, len(todo))

    def test_a_complete_set_needs_no_tags(self):
        self.assertEqual([], rs.missing(COMPLETE))

    def test_a_blank_template_is_missing(self):
        self.assertEqual(["sheet name logic"], rs.missing(dict(COMPLETE, sheet_name="  ")))



class Sections(unittest.TestCase):
    def test_a_section_is_stored_beside_the_server_without_disturbing_it(self):
        text = ps.with_server(None, "http://s:5151")
        text = ps.with_section(text, "rls", COMPLETE)
        self.assertEqual("http://s:5151", ps.server_of(text))
        self.assertEqual(COMPLETE, ps.get_section(text, "rls"))

    def test_writing_the_server_afterwards_keeps_the_section(self):
        text = ps.with_section(None, "rls", COMPLETE)
        self.assertEqual(COMPLETE, ps.get_section(ps.with_server(text, "http://s:5151"), "rls"))

    def test_a_missing_or_wrongly_typed_section_is_none(self):
        self.assertIsNone(ps.get_section(None, "rls"))
        self.assertIsNone(ps.get_section(json.dumps({"rls": 5}), "rls"))

    def test_removing_a_section_leaves_the_rest(self):
        text = ps.with_section(ps.with_server(None, "x"), "rls", COMPLETE)
        gone = ps.with_section(text, "rls", None)
        self.assertIsNone(ps.get_section(gone, "rls"))
        self.assertEqual("x", ps.server_of(gone))


if __name__ == "__main__":
    unittest.main()

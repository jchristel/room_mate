"""The settings hand-off (`room_m.rls_settings`, `room_m.project_settings` sections), pure half. CPython."""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import project_settings as ps  # noqa: E402
from room_m import rls_settings as rs  # noqa: E402
from room_m import server_config as sc  # noqa: E402

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

    def test_it_agrees_with_the_page(self):
        """The page's `missing` (model.ts) and this one ask the same question: same size keys, same fields."""
        with open(os.path.join(os.path.dirname(__file__), "..", "..", "src-js", "model-settings", "model.ts")) as f:
            ts = f.read()
        for key in ("plan_view_type", "elevation_view_type", "schedule_base", "sheet_number", "sheet_name", "view_name"):
            self.assertIn(key, ts)
        self.assertIn('["a0", "a1", "a2", "a3"]', ts)


class Urls(unittest.TestCase):
    def setUp(self):
        self.addCleanup(sc.set_project_reader, None)
        sc.set_project_reader(lambda: "192.168.0.97")

    def test_the_page_is_on_the_server_in_force_with_both_ids_escaped(self):
        url = rs.page_url("House A", "Model 1/x", {}, "/nonexistent/server.json")
        self.assertEqual("http://192.168.0.97:5151/model-settings/?project=House%20A&model=Model%201%2Fx", url)

    def test_the_draft_paths_are_paths_not_addresses(self):
        self.assertEqual("/projects/House%20A/modelsettings", rs.draft_path("House A"))
        self.assertEqual("/projects/p/modelsettings?model=a%20b", rs.draft_get_path("p", "a b"))

    def test_no_server_set_is_reported_not_guessed(self):
        sc.set_project_reader(None)
        with self.assertRaises(sc.ServerNotSet):
            rs.page_url("p", "m", {}, "/nonexistent/server.json")


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


class Draft(unittest.TestCase):
    def test_the_body_always_has_all_three_parts(self):
        self.assertEqual({"model_id": "m", "settings": {}, "options": {}}, rs.draft_body("m", None, None))

    def test_a_server_answer_that_is_not_a_draft_is_none(self):
        self.assertIsNone(rs.parse_draft("<html>"))
        self.assertIsNone(rs.parse_draft("[1]"))
        self.assertEqual({"saved": True}, rs.parse_draft('{"saved": true}'))


class Source(unittest.TestCase):
    def test_the_modules_ironpython_loads_are_ascii(self):
        here = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
        for name in ("rls_settings.py", "project_settings.py", "room_mate.py", "post_common.py"):
            with open(os.path.join(here, name), "rb") as f:
                f.read().decode("ascii")


if __name__ == "__main__":
    unittest.main()

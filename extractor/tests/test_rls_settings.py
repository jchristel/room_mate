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
        self.assertIn("a title block for at least one sheet size", todo)
        self.assertIn("schedule base", todo)
        self.assertEqual(1 + 6, len(todo))

    def test_one_title_block_is_enough(self):
        only_a3 = dict(COMPLETE, sheet_sizes={"a3": "tb"})
        self.assertEqual([], rs.missing(only_a3))

    def test_a_complete_set_needs_no_tags(self):
        self.assertEqual([], rs.missing(COMPLETE))

    def test_the_3d_view_type_is_not_demanded_of_every_project(self):
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


class Validate(unittest.TestCase):
    OPTIONS = {
        "title_blocks": ["TB : A1"],
        "plan_view_types": ["Plan"],
        "elevation_view_types": ["Elev"],
        "three_d_view_types": ["3D"],
        "viewport_types": ["Title w Line"],
        "schedules": ["Sched"],
        "room_tags": ["RT"],
        "independent_tags": ["IT"],
        "room_parameters": ["Number", "Name"],
    }

    def settings(self, **over):
        s = {
            "sheet_sizes": {"a0": "TB : A1", "a1": "TB : A1", "a2": "TB : A1", "a3": "TB : A1"},
            "plan_view_type": "Plan", "elevation_view_type": "Elev", "schedule_base": "Sched",
            "room_tag": "RT", "independent_tag": "IT",
            "sheet_number": "{Number}", "sheet_name": "{Name}", "view_name": "{Number}",
        }
        s.update(over)
        return s

    def test_a_chosen_3d_view_type_the_model_lacks_is_an_error(self):
        errors, _ = rs.validate(self.settings(three_d_view_type="Old"), self.OPTIONS)
        self.assertEqual(["3D view type 'Old' is not in this model"], errors)

    def test_a_chosen_viewport_type_the_model_lacks_is_an_error(self):
        errors, _ = rs.validate(self.settings(viewport_type="Old"), self.OPTIONS)
        self.assertEqual(["viewport type 'Old' is not in this model"], errors)
        self.assertEqual(([], []), rs.validate(self.settings(viewport_type="Title w Line"), self.OPTIONS))

    def test_good_settings_have_nothing_to_report(self):
        self.assertEqual(([], []), rs.validate(self.settings(), self.OPTIONS))

    def test_a_stored_value_the_model_lacks_is_an_error_and_a_stale_tag_only_a_warning(self):
        errors, warnings = rs.validate(self.settings(schedule_base="Old", room_tag="Gone"), self.OPTIONS)
        self.assertEqual(["base schedule 'Old' is not in this model"], errors)
        self.assertEqual(["room tag 'Gone' is not in this model"], warnings)

    def test_a_title_block_per_size_is_checked(self):
        s = self.settings()
        s["sheet_sizes"]["a2"] = "Nope"
        self.assertEqual(["title block for A2 'Nope' is not in this model"], rs.validate(s, self.OPTIONS)[0])

    def test_a_list_the_model_did_not_offer_is_not_checked(self):
        options = dict(self.OPTIONS)
        del options["schedules"]
        self.assertEqual(([], []), rs.validate(self.settings(schedule_base="Old"), options))

    def test_template_words_and_numbering_styles_are_checked(self):
        errors, _w = rs.validate(self.settings(sheet_name="{Wing}", plan_detail_numbering="roman"), self.OPTIONS)
        self.assertEqual(2, len(errors))
        self.assertIn("{Wing}", errors[0])

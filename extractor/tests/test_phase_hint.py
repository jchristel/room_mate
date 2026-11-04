"""Which phase the server holds (`room_m.phase_hint`) and the window's default (`export_form.default_phase`)."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import export_form as form  # noqa: E402
from room_m import phase_hint  # noqa: E402


PROJECTS = [
    {"id": "p", "name": "P", "model_phases": {"A": "New Construction", "B": "New Construction", "C": "Future"}},
    {"id": "q", "name": "Q"},
]


class PhasesHeld(unittest.TestCase):
    def test_phases_come_in_model_order_without_repeats_or_blanks(self):
        self.assertEqual(["New Construction"], phase_hint.phases_held(PROJECTS, "p", ["A", "B", "D"]))
        self.assertEqual(["Future", "New Construction"], phase_hint.phases_held(PROJECTS, "p", ["C", "A"]))

    def test_a_project_with_nothing_stored_or_unknown_is_no_hint_and_never_raises(self):
        self.assertEqual([], phase_hint.phases_held(PROJECTS, "q", ["A"]))
        self.assertEqual([], phase_hint.phases_held(PROJECTS, "nope", ["A"]))
        self.assertEqual([], phase_hint.phases_held(None, "p", ["A"]))


class DefaultPhase(unittest.TestCase):
    phases = ["Existing", "New Construction", "Future"]

    def test_the_phase_the_server_holds_is_preselected(self):
        phase, why = form.default_phase(self.phases, ["New Construction"])
        self.assertEqual("New Construction", phase)
        self.assertIn("last push", why)

    def test_a_first_push_offers_the_first_phase(self):
        phase, why = form.default_phase(self.phases, [])
        self.assertEqual("Existing", phase)
        self.assertIn("First push", why)

    def test_a_phase_this_model_lacks_falls_back_to_the_first_and_says_what_the_server_holds(self):
        phase, why = form.default_phase(self.phases, ["Demolition"])
        self.assertEqual("Existing", phase)
        self.assertIn("Demolition", why)

    def test_no_phases_no_default(self):
        self.assertEqual((None, ""), form.default_phase([], ["x"]))


class Remembered(unittest.TestCase):
    projects = [{"id": "a", "name": "A"}, {"id": "b", "name": "B"}]

    def test_the_last_project_entities_and_levels_come_back(self):
        saved = form.to_save(self.projects[1], ("rooms", "doors"), ["L1", "L2"])
        got = form.remembered(saved, self.projects, ["L1", "L2", "L3"])
        self.assertEqual(1, got["project"])
        self.assertEqual({"cb_rooms", "cb_doors"}, got["entities"])
        self.assertEqual(["L1", "L2"], got["levels"])

    def test_the_whole_model_is_remembered_as_none(self):
        saved = form.to_save(self.projects[0], ("rooms",), None)
        self.assertIsNone(form.remembered(saved, self.projects, ["L1"])["levels"])

    def test_what_no_longer_exists_is_dropped(self):
        saved = form.to_save({"id": "gone"}, ("rooms", "nonsense"), ["Old"])
        got = form.remembered(saved, self.projects, ["L1"])
        self.assertIsNone(got["project"])
        self.assertEqual({"cb_rooms"}, got["entities"])
        self.assertIsNone(got["levels"])

    def test_nothing_saved_starts_empty(self):
        got = form.remembered({}, self.projects, [])
        self.assertEqual((None, set(), None), (got["project"], got["entities"], got["levels"]))

    def test_check_all_ticks_everything_but_ffe_by_space(self):
        everything = form.entities_all()
        self.assertIn("cb_ffe", everything)
        self.assertNotIn("cb_ffe_by_space", everything)
        self.assertEqual([], [p for p in form.problems(everything, {"id": "p"}, "x", True, []) if "FF&E" in p])


if __name__ == "__main__":
    unittest.main()

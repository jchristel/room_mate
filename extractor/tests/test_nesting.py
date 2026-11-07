"""What is nested in what (`room_m.utils.nesting`), and how it and the export window's nested choice reach the wire
(`post_entity`, `post_ffe`, `export_form.apply_nested`). Stdlib unittest, CPython."""

import os
import sys
import unittest

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
sys.path.insert(0, os.path.join(ROOM_M, ".."))

from room_m import export_form as form  # noqa: E402
from room_m.utils import nesting  # noqa: E402


INFO = {
    "11": {"category": "Doors", "family_name": "Leaf", "type_name": "820"},
    "12": {"category": "Generic Models", "family_name": "Handle", "type_name": None},
    "21": {"category": "Specialty Equipment", "family_name": "Rail", "type_name": "HYTP-100"},
}


class Nesting(unittest.TestCase):
    def test_a_host_lists_every_depth_flat_each_with_its_own_parent(self):
        found = nesting.nesting_map({"11": "1", "12": "11", "21": "2"}, INFO)
        self.assertEqual(
            [{"id": "11", "parent_id": "1", "category": "Doors", "family_name": "Leaf", "type_name": "820"},
             {"id": "12", "parent_id": "11", "category": "Generic Models", "family_name": "Handle"}],
            found["1"])
        self.assertEqual(["12"], [c["id"] for c in found["11"]], "a nested host lists its own as well")
        self.assertEqual("HYTP-100", found["2"][0]["type_name"])
        self.assertNotIn("12", found, "an instance with nothing in it is not a host")

    def test_a_cycle_is_walked_once(self):
        self.assertEqual(["2"], [c["id"] for c in nesting.descendants({"1": ["2"], "2": ["1"]}, {}, "1")])


class NestedChoice(unittest.TestCase):
    def test_the_defaults_are_what_the_producer_did_before(self):
        self.assertEqual({"ffe", "ffe_by_space"}, form.nested_default())

    def test_including_nested_openings_forgets_their_ids(self):
        c = form.apply_nested("doors", {"nested_ids": {"5"}}, True)
        self.assertEqual(set(), c["nested_ids"])
        self.assertEqual({"5"}, form.apply_nested("windows", {"nested_ids": {"5"}}, False)["nested_ids"])

    def test_leaving_out_ffe_components_is_a_flag_and_other_entities_are_untouched(self):
        self.assertTrue(form.apply_nested("ffe", {}, False)["drop_components"])
        self.assertFalse(form.apply_nested("ffe_by_space", {}, True)["drop_components"])
        self.assertEqual({}, form.apply_nested("rooms", {}, False))
        self.assertIsNone(form.apply_nested("doors", None, True))

    def test_the_choice_is_remembered_and_an_old_save_gets_the_defaults(self):
        saved = form.to_save({"id": "p"}, ["doors"], None, {"doors"})
        self.assertEqual({"cb_nested_doors"}, form.remembered(saved, [{"id": "p"}], [])["nested"])
        self.assertEqual({"cb_nested_ffe", "cb_nested_ffe_by_space"}, form.remembered({}, [], [])["nested"])
        self.assertEqual(set(), form.remembered({"nested": []}, [], [])["nested"])


class OnTheWire(unittest.TestCase):
    def source(self, name):
        with open(os.path.join(ROOM_M, name)) as f:
            return f.read()

    def test_every_host_entity_reads_the_nesting_and_sends_it(self):
        for name in ("doors", "windows", "ffe"):
            text = self.source("exporters/%s.py" % name)
            self.assertIn("nesting = read_nesting(selected_doc)", text, name)
            self.assertIn('"nesting": nesting', text, name)
        self.assertIn('"sub_components": (nesting or {}).get(opening_id, [])', self.source("post_entity.py"))
        self.assertIn('"sub_components": (contribution.get("nesting") or {}).get(item_id, [])', self.source("post_ffe.py"))
        for name in ("post_doors.py", "post_windows.py"):
            self.assertIn('contribution.get("nesting")', self.source(name), name)

    def test_the_push_drops_what_the_choice_leaves_out_on_both_paths(self):
        text = self.source("post_entity.py")
        self.assertEqual(2, text.count("is_left_out_nested("), "the buffered and the streamed path")
        self.assertIn("contribution = export_form.apply_nested(entity, contribution, entity in nested)", self.source("room_mate.py"))

    def test_the_rule_itself(self):
        is_left_out_nested = form.is_left_out_nested

        self.assertTrue(is_left_out_nested({"id": "5"}, {}, {"5"}))
        self.assertFalse(is_left_out_nested({"id": "6", "super_component_id": "1"}, {}, set()))
        self.assertTrue(is_left_out_nested({"id": "6", "super_component_id": "1"}, {"drop_components": True}, set()))
        self.assertFalse(is_left_out_nested({"id": "7", "super_component_id": None}, {"drop_components": True}, set()))


if __name__ == "__main__":
    unittest.main()

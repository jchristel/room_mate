"""The export window's choices (`room_m.export_form`), the level scope on the wire (`utils.phase_filter`) and the XAML. Stdlib unittest, CPython."""

import os
import re
import sys
import unittest
import xml.etree.ElementTree as ET

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
sys.path.insert(0, os.path.join(ROOM_M, ".."))

from room_m import export_form as form  # noqa: E402
from room_m.utils import phase_filter as pf  # noqa: E402

X = "{http://schemas.microsoft.com/winfx/2006/xaml}"
NS = "{http://schemas.microsoft.com/winfx/2006/xaml/presentation}"


def level(i, name, elevation):
    return {"id": i, "name": name, "elevation": elevation}


class Entities(unittest.TestCase):
    def test_the_ticked_entities_come_back_in_the_windows_order(self):
        self.assertEqual(("rooms", "floors"), form.entities_ticked({"cb_floors", "cb_rooms"}))

    def test_the_keys_are_the_ones_the_run_driver_dispatches_on(self):
        with open(os.path.join(ROOM_M, "room_mate.py")) as f:
            text = f.read()
        for _name, key, _label in form.ENTITIES:
            self.assertRegex(text, r'= "%s"' % key)


class Levels(unittest.TestCase):
    A = [level("1", "Ground", 0.0), level("2", "Level 1", 12.0), level("3", "Roof", 40.0)]
    B = [level("9", "Level 1", 11.5), level("8", "Basement", -10.0)]

    def test_names_merge_across_models_lowest_first(self):
        self.assertEqual(["Basement", "Ground", "Level 1", "Roof"], form.level_names([self.A, self.B]))

    def test_each_model_gets_its_own_ids_for_the_chosen_names(self):
        self.assertEqual(["2"], form.scope_for_doc(self.A, ["Level 1"]))
        self.assertEqual(["9"], form.scope_for_doc(self.B, ["Level 1"]))

    def test_a_model_with_none_of_the_names_has_an_empty_scope(self):
        self.assertEqual([], form.scope_for_doc(self.B, ["Roof"]))


class Problems(unittest.TestCase):
    def test_a_good_choice_has_no_problems(self):
        self.assertEqual([], form.problems({"cb_rooms"}, {"id": "p"}, "New Construction", True, []))

    def test_each_missing_choice_is_said(self):
        lines = form.problems(set(), None, None, False, [])
        self.assertEqual(4, len(lines), lines)

    def test_ffe_by_room_and_by_space_are_exclusive(self):
        lines = form.problems({"cb_ffe", "cb_ffe_by_space"}, {"id": "p"}, "x", True, [])
        self.assertTrue(any("OR" in l for l in lines))

    def test_chosen_levels_satisfy_the_scope(self):
        self.assertEqual([], form.problems({"cb_rooms"}, {"id": "p"}, "x", False, ["Level 1"]))


class ScopeOnTheWire(unittest.TestCase):
    def test_an_unscoped_block_passes_everything(self):
        self.assertTrue(pf.in_scope({"level_id": "5"}, {"id": "m"}))
        self.assertTrue(pf.in_scope({"level_id": "5"}, None))

    def test_a_scoped_block_passes_only_its_levels(self):
        block = {"id": "m", "scope": {"levels": ["1", "2"]}}
        self.assertTrue(pf.in_scope({"level_id": "2"}, block))
        self.assertFalse(pf.in_scope({"level_id": "3"}, block))
        self.assertTrue(pf.in_scope({"level_id": 2}, block), "ids are compared as text")

    def test_an_element_with_no_level_is_not_in_a_scope(self):
        self.assertFalse(pf.in_scope({}, {"scope": {"levels": ["1"]}}))

    def test_a_scoped_push_is_known_from_its_blocks(self):
        self.assertTrue(pf.is_scoped([{"id": "a"}, {"id": "b", "scope": {"levels": ["1"]}}]))
        self.assertFalse(pf.is_scoped([{"id": "a"}]))

    def test_every_push_module_applies_the_scope_after_the_phase(self):
        for name in ("post_rooms.py", "post_spaces.py", "post_surfaces.py", "post_entity.py"):
            with open(os.path.join(ROOM_M, name)) as f:
                text = f.read()
            self.assertIn("in_scope(", text, name)
            self.assertLess(text.rindex("in_selected_phase("), text.rindex("in_scope("), name)


class LevelFilterWiring(unittest.TestCase):
    """The level choice reaches duHast: each exporter takes `level_ids` and hands it to the duHast
    export, and the count guards compare against the same filtered set."""

    def source(self, name):
        with open(os.path.join(ROOM_M, name)) as f:
            return f.read()

    def test_every_exporter_takes_level_ids_and_passes_them_to_duhast(self):
        for name, call in (("rooms", "get_all_room_data"), ("doors", "get_all_door_data"), ("windows", "get_all_window_data"),
                           ("spaces", "get_all_space_data"), ("ffe", "get_all_item_data")):
            text = self.source("exporters/%s.py" % name)
            self.assertIn("level_ids=None", text, name)
            self.assertRegex(text, r"%s\(selected_doc, \*\*level_kwargs\(level_ids\)\)" % call)
        for name in ("ceilings", "floors"):
            self.assertIn("level_ids=level_ids", self.source("exporters/%s.py" % name))
        self.assertIn("spec.export(selected_doc, **level_kwargs(level_ids))", self.source("exporters/surfaces.py"))

    def test_the_count_guards_compare_against_the_filtered_set(self):
        self.assertIn("is_on_levels(e, level_values)", self.source("exporters/surfaces.py"))
        self.assertIn("is_on_levels(s, level_values)", self.source("exporters/spaces.py"))

    def test_an_unscoped_push_sends_duhast_no_new_argument_so_an_older_duhast_still_works(self):
        level_kwargs = form.level_kwargs

        self.assertEqual({}, level_kwargs(None))
        self.assertEqual({"level_ids": ["1"]}, level_kwargs(["1"]))

    def test_the_run_driver_hands_the_level_ids_to_every_exporter(self):
        self.assertIn("exporter.export_model(selected_doc, phase_name, return_value, level_ids=level_ids)", self.source("room_mate.py"))
        self.assertIn('block["scope"] = {"levels": list(level_ids)}', self.source("room_mate.py"))


class Xaml(unittest.TestCase):
    def setUp(self):
        self.root = ET.parse(os.path.join(ROOM_M, "export.xaml")).getroot()
        self.names = [e.get(X + "Name") for e in self.root.iter() if e.get(X + "Name")]

    def test_it_has_no_class_and_no_grid_control(self):
        self.assertIsNone(self.root.get(X + "Class"))
        tags = set(e.tag for e in self.root.iter())
        self.assertNotIn(NS + "DataGrid", tags)
        self.assertNotIn(NS + "ListView", tags)

    def test_every_entity_has_a_check_box_and_the_controls_the_window_reads_exist(self):
        for name, _key, _label in form.ENTITIES:
            self.assertEqual(1, self.names.count(name), name)
        for name in ("cb_project", "cb_phase", "rb_whole", "rb_levels", "sp_levels", "tx_status", "tx_models", "bt_export"):
            self.assertEqual(1, self.names.count(name), name)

    def test_every_handler_in_the_xaml_is_a_method_of_the_window(self):
        with open(os.path.join(ROOM_M, "export_window.py")) as f:
            source = f.read()
        handlers = set()
        for e in self.root.iter():
            for attribute in ("Click", "SelectionChanged", "Checked", "Unchecked", "TextChanged"):
                if e.get(attribute):
                    handlers.add(e.get(attribute))
        self.assertTrue(handlers)
        for h in handlers:
            self.assertTrue(re.search(r"def {}\(self, sender, args\)".format(h), source), h)


class Panel(unittest.TestCase):
    EXT = os.path.join(ROOM_M, "..", "RoomMate.extension", "duHast.tab")

    def test_the_new_buttons_are_on_a_roommate_panel_not_the_export_panel(self):
        self.assertTrue(os.path.isdir(os.path.join(self.EXT, "RoomMate.panel", "ExportAll.pushbutton")))
        with open(os.path.join(self.EXT, "bundle.yaml")) as f:
            layout = f.read()
        self.assertRegex(layout, r"layout:\s*\n\s*- RoomMate\s*\n\s*- Export")
        for button in ("ExportAll", "ApplyTasks", "RlsSettings", "RlsCreate", "Server"):
            self.assertTrue(os.path.isdir(os.path.join(self.EXT, "RoomMate.panel", button + ".pushbutton")), button)
            self.assertFalse(os.path.isdir(os.path.join(self.EXT, "Export.panel", button + ".pushbutton")), button)


if __name__ == "__main__":
    unittest.main()

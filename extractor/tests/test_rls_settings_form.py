"""The RLS settings form (`room_m.rls_settings_form`) and its XAML. Stdlib unittest, CPython, no Revit or WPF."""

import os
import re
import sys
import unittest
import xml.etree.ElementTree as ET

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
sys.path.insert(0, os.path.join(ROOM_M, ".."))

from room_m import rls_settings_form as form  # noqa: E402

X = "{http://schemas.microsoft.com/winfx/2006/xaml}"
NS = "{http://schemas.microsoft.com/winfx/2006/xaml/presentation}"


class Choices(unittest.TestCase):
    def test_a_first_entry_means_not_set_and_nothing_stored_selects_it(self):
        labels, values, selected = form.choices("", ["A", "B"])
        self.assertEqual([form.NOT_SET, "A", "B"], labels)
        self.assertEqual(["", "A", "B"], values)
        self.assertEqual(0, selected)

    def test_a_stored_value_the_model_has_is_selected(self):
        self.assertEqual(2, form.choices("B", ["A", "B"])[2])

    def test_a_stored_value_the_model_lost_stays_visible_and_marked(self):
        labels, values, selected = form.choices("Old", ["A"])
        self.assertEqual("Old" + form.NOT_IN_MODEL, labels[selected])
        self.assertEqual("Old", values[selected], "saving keeps it rather than silently dropping it")

    def test_a_list_revit_did_not_offer_is_just_not_set(self):
        self.assertEqual([form.NOT_SET], form.choices("", None)[0])


class Tokens(unittest.TestCase):
    def test_a_picked_parameter_is_added_in_braces(self):
        params = ["Number", "Name"]
        self.assertEqual("RLS-{Name}", form.add_token("RLS-", params, 2))

    def test_the_prompt_and_nothing_change_nothing(self):
        for index in (0, None, 9):
            self.assertEqual("x", form.add_token("x", ["Number"], index))

    def test_the_labels_start_with_the_prompt(self):
        self.assertEqual([form.TOKEN_PROMPT, "Number"], form.token_labels(["Number"]))


class ToSettings(unittest.TestCase):
    def test_the_form_values_are_set_and_unknown_keys_survive(self):
        existing = {"future": {"x": 1}, "sheet_sizes": {"a0": "old", "a9": "keep"}}
        combos = dict((name, "v-" + name) for name, _o, _p in form.COMBOS)
        texts = {"tb_sheet_number": " RLS-{Number} ", "tb_sheet_name": "{Name}", "tb_view_name": "{Number}"}
        s = form.to_settings(existing, combos, texts)
        self.assertEqual("v-cb_a0", s["sheet_sizes"]["a0"])
        self.assertEqual("keep", s["sheet_sizes"]["a9"])
        self.assertEqual({"x": 1}, s["future"])
        self.assertEqual("RLS-{Number}", s["sheet_number"], "trimmed")
        self.assertEqual("v-cb_schedule_base", s["schedule_base"])

    def test_the_input_is_not_changed(self):
        existing = {"sheet_sizes": {"a0": "old"}}
        form.to_settings(existing, {"cb_a0": "new"}, {})
        self.assertEqual("old", existing["sheet_sizes"]["a0"])

    def test_it_agrees_with_the_settings_the_web_page_and_planner_use(self):
        from room_m import rls_settings

        combos = dict((name, "x") for name, _o, _p in form.COMBOS)
        texts = dict((name, "{Number}") for name, _k in form.TEXTS)
        self.assertEqual([], rls_settings.missing(form.to_settings({}, combos, texts)))


class Problems(unittest.TestCase):
    PARAMS = ["Number", "Name"]

    def test_an_unknown_token_is_found_ignoring_case(self):
        self.assertEqual(["Nmae"], form.unknown_tokens("{number}-{Nmae}", self.PARAMS))

    def test_no_parameter_list_means_no_complaint(self):
        self.assertEqual([], form.unknown_tokens("{Anything}", []))
        self.assertEqual([], form.unknown_tokens("{Anything}", None))

    def test_the_status_lines_say_what_is_missing_and_what_is_wrong(self):
        lines = form.problems({"sheet_number": "{Wing}"}, self.PARAMS)
        self.assertTrue(lines[0].startswith("Still to choose: "))
        self.assertIn("{Wing}", lines[1])

    def test_a_complete_form_has_nothing_to_say(self):
        combos = dict((name, "x") for name, _o, _p in form.COMBOS)
        texts = dict((name, "{Number}") for name, _k in form.TEXTS)
        self.assertEqual([], form.problems(form.to_settings({}, combos, texts), self.PARAMS))


class Xaml(unittest.TestCase):
    def setUp(self):
        self.root = ET.parse(os.path.join(ROOM_M, "rls_settings.xaml")).getroot()
        self.names = [e.get(X + "Name") for e in self.root.iter() if e.get(X + "Name")]

    def test_it_is_well_formed_has_no_class_and_no_grid_control(self):
        self.assertEqual(NS + "Window", self.root.tag)
        self.assertIsNone(self.root.get(X + "Class"), "no x:Class: the window is loaded from the file")
        tags = set(e.tag for e in self.root.iter())
        self.assertNotIn(NS + "DataGrid", tags, "the control that crashed Revit")
        self.assertNotIn(NS + "ListView", tags)

    def test_every_control_the_form_names_exists_once(self):
        wanted = [n for n, _o, _p in form.COMBOS] + [n for n, _k in form.TEXTS] + ["tk_" + n[3:] for n, _k in form.TEXTS]
        for name in wanted + ["tx_model", "tx_status"]:
            self.assertEqual(1, self.names.count(name), name)

    def test_every_handler_named_in_the_xaml_is_a_method_of_the_window(self):
        with open(os.path.join(ROOM_M, "rls_settings_window.py")) as f:
            source = f.read()
        handlers = set()
        for e in self.root.iter():
            for attribute in ("Click", "SelectionChanged", "TextChanged"):
                if e.get(attribute):
                    handlers.add(e.get(attribute))
        self.assertTrue(handlers)
        for h in handlers:
            self.assertTrue(re.search(r"def {}\(self, sender, args\)".format(h), source), h)

    def test_every_field_reports_its_changes_so_the_status_line_stays_true(self):
        handlers = dict((e.get(X + "Name"), e) for e in self.root.iter() if e.get(X + "Name"))
        for name, _o, _p in form.COMBOS:
            self.assertEqual("on_change", handlers[name].get("SelectionChanged"), name)
        for name, _k in form.TEXTS:
            self.assertEqual("on_change", handlers[name].get("TextChanged"), name)


if __name__ == "__main__":
    unittest.main()

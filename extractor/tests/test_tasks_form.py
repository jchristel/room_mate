"""The task chooser's rules (`room_m.tasks_form`) and its XAML. Stdlib unittest, CPython."""

import os
import re
import sys
import unittest
import xml.etree.ElementTree as ET

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
sys.path.insert(0, os.path.join(ROOM_M, ".."))

from room_m import tasks_form as form  # noqa: E402

X = "{http://schemas.microsoft.com/winfx/2006/xaml}"
NS = "{http://schemas.microsoft.com/winfx/2006/xaml/presentation}"

LEVELS = {"1": "LEVEL 1", "2": "LEVEL 2"}


def level_of(room):
    return LEVELS.get(room.get("room_id"))


def summary(task_id, kind, number, name, room_id, blueprint=None):
    s = {"id": task_id, "kind": kind, "room": {"room_id": room_id, "number": number, "name": name}}
    if blueprint:
        s["blueprint"] = {"id": "b", "name": blueprint, "revision": 1}
    return s


SUMMARIES = [
    summary("t3", "populate_room", "02.01", "OFFICE", "2", "Ward"),
    summary("t1", "create_rls", "01.09", "STUDY", "1"),
    summary("t2", "create_rls", "01.10", "BED", "9"),
]


class Rows(unittest.TestCase):
    def test_a_row_reads_room_kind_and_level_and_sorts_by_level_with_unknown_last(self):
        rows = form.rows(SUMMARIES, level_of)
        self.assertEqual(["t1", "t3", "t2"], [r["id"] for r in rows])
        self.assertEqual("01.09 STUDY  -  Create RLS  -  LEVEL 1", rows[0]["text"])
        self.assertEqual(form.NO_LEVEL, rows[2]["level"])
        self.assertIn("blueprint Ward", rows[1]["text"])

    def test_the_level_choices_list_each_level_once_with_all_first(self):
        self.assertEqual(
            [form.ALL_LEVELS, "LEVEL 1", "LEVEL 2", form.NO_LEVEL], form.level_choices(form.rows(SUMMARIES, level_of)))


class Filters(unittest.TestCase):
    def ids(self, text="", kind=form.KIND_ALL, level=form.ALL_LEVELS):
        return [r["id"] for r in form.rows(SUMMARIES, level_of) if form.shown(r, text, kind, level)]

    def test_no_filter_shows_everything(self):
        self.assertEqual(["t1", "t3", "t2"], self.ids())

    def test_text_matches_number_name_blueprint_or_id_ignoring_case(self):
        self.assertEqual(["t1"], self.ids("study"))
        self.assertEqual(["t3"], self.ids("WARD"))
        self.assertEqual(["t2"], self.ids("t2"))
        self.assertEqual(["t1", "t2"], self.ids("01."))

    def test_kind_splits_create_rls_from_the_rest(self):
        self.assertEqual(["t1", "t2"], self.ids(kind=form.KIND_RLS))
        self.assertEqual(["t3"], self.ids(kind=form.KIND_OTHER))

    def test_level_and_kind_combine(self):
        self.assertEqual(["t1"], self.ids(level="LEVEL 1"))
        self.assertEqual([], self.ids(level="LEVEL 1", kind=form.KIND_OTHER))


class Status(unittest.TestCase):
    def test_nothing_ticked_stops_a_run(self):
        self.assertTrue(form.problems(0))
        self.assertEqual([], form.problems(2))


class Xaml(unittest.TestCase):
    def setUp(self):
        self.root = ET.parse(os.path.join(ROOM_M, "tasks.xaml")).getroot()
        self.names = [e.get(X + "Name") for e in self.root.iter() if e.get(X + "Name")]

    def test_it_has_no_class_and_no_grid_control(self):
        self.assertIsNone(self.root.get(X + "Class"))
        tags = set(e.tag for e in self.root.iter())
        self.assertNotIn(NS + "DataGrid", tags)
        self.assertNotIn(NS + "ListView", tags)

    def test_the_controls_the_window_reads_exist(self):
        for name in ("cb_project", "tb_filter", "cb_kind", "cb_level", "sp_tasks", "tx_status", "tx_model", "bt_run"):
            self.assertEqual(1, self.names.count(name), name)

    def test_every_handler_in_the_xaml_is_a_method_of_the_window(self):
        with open(os.path.join(ROOM_M, "tasks_window.py")) as f:
            source = f.read()
        handlers = set()
        for e in self.root.iter():
            for attribute in ("Click", "SelectionChanged", "Checked", "Unchecked", "TextChanged"):
                if e.get(attribute):
                    handlers.add(e.get(attribute))
        self.assertTrue(handlers)
        for h in handlers:
            self.assertTrue(re.search(r"def {}\(self, sender, args\)".format(h), source), h)


if __name__ == "__main__":
    unittest.main()

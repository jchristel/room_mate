"""The settings kept in the Revit project file (`room_m.project_settings`), pure half. CPython."""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import project_settings as ps  # noqa: E402


class Text(unittest.TestCase):
    def test_nothing_or_rubbish_is_no_settings(self):
        for text in (None, "", "{not json", "[1, 2]", "5"):
            self.assertEqual({}, ps.parse(text), repr(text))
            self.assertIsNone(ps.server_of(text), repr(text))

    def test_the_server_is_written_and_read_back(self):
        text = ps.with_server(None, "http://192.168.0.97:5151")
        self.assertEqual("http://192.168.0.97:5151", ps.server_of(text))
        self.assertEqual(ps.VERSION, json.loads(text)["version"])

    def test_writing_the_server_keeps_every_other_setting(self):
        later = json.dumps({"version": 3, "sheet_sizes": {"a1": "A1 sheet"}, "server": "http://old:5151"})
        text = json.loads(ps.with_server(later, "http://new:5151"))
        self.assertEqual("http://new:5151", text["server"])
        self.assertEqual({"a1": "A1 sheet"}, text["sheet_sizes"])
        self.assertEqual(3, text["version"], "a newer version number is not lowered")

    def test_removing_the_server_leaves_the_rest(self):
        text = json.loads(ps.with_server(json.dumps({"server": "x", "other": 1}), None))
        self.assertNotIn("server", text)
        self.assertEqual(1, text["other"])

    def test_a_blank_server_is_none(self):
        self.assertIsNone(ps.server_of(json.dumps({"server": "  "})))
        self.assertIsNone(ps.server_of(json.dumps({"server": 5})))

    def test_without_revit_nothing_is_read_and_nothing_raises(self):
        self.assertIsNone(ps.read_text(None))
        self.assertIsNone(ps.read_text(object()), "no Revit libraries here: the next source is used")
        self.assertIsNone(ps.server_of_active_document())
        self.assertFalse(ps.write_text(None, "{}")[0])


if __name__ == "__main__":
    unittest.main()

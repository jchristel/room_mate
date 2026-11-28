"""How an item's Revit facts reach the wire (`post_ffe.translate_item`, `utils.items.item_facts`): the family's
placement type and the instance's UniqueId, both from duHast's `get_item_facts`, and what an older duHast that
returns neither does. Stdlib unittest, CPython."""

import os
import sys
import types
import unittest

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
sys.path.insert(0, os.path.join(ROOM_M, ".."))

# `post_common` loads .NET assemblies at import (the HTTP client); CPython has none, and nothing here sends a request.
from unittest import mock  # noqa: E402

for _name in ("clr", "System", "System.IO", "System.IO.Compression", "System.Net", "System.Net.Http",
              "System.Net.Http.Headers", "System.Text"):
    sys.modules.setdefault(_name, mock.MagicMock())

from room_m import post_ffe  # noqa: E402


def export_item(item_id="42"):
    return {
        "instance_properties": {"id": item_id},
        "type_properties": {"id": "7", "name": "Queen"},
        "level": {"id": "1"},
        "location_point": {"translation_coord": {"x": 1000.0, "y": 2000.0}},
    }


def contribution(facts):
    return {"facts": {"42": facts}}


class ItemFactsOnTheWire(unittest.TestCase):
    def test_placement_type_and_unique_id_ride_the_item(self):
        facts = {"room": "9", "category": "Furniture", "family_name": "Bed", "host": None,
                 "placement_type": "OneLevelBased", "unique_id": "abc-0001"}
        out = post_ffe.translate_item(export_item(), contribution(facts))
        self.assertEqual("OneLevelBased", out["placement_type"])
        self.assertEqual("abc-0001", out["unique_id"])

    def test_an_older_duhast_leaves_both_absent_rather_than_guessing(self):
        facts = {"room": "9", "category": "Furniture", "family_name": "Bed", "host": None}
        out = post_ffe.translate_item(export_item(), contribution(facts))
        self.assertIsNone(out["placement_type"])
        self.assertIsNone(out["unique_id"])

    def test_an_item_with_no_facts_at_all_still_translates(self):
        out = post_ffe.translate_item(export_item(), {"facts": {}})
        self.assertIsNone(out["placement_type"])
        self.assertIsNone(out["unique_id"])


class ItemFactsReading(unittest.TestCase):
    """`item_facts` over a stand-in for duHast's `get_item_facts`, which Revit-less CPython cannot import."""

    def run_with(self, found):
        for name, attrs in (("duHast", {}), ("duHast.Revit", {}), ("duHast.Revit.Family", {}),
                            ("duHast.Revit.Family.family_instance_facts", {"get_item_facts": None}),
                            ("room_m.utils.generic", {"elements_in_phase": None, "phase_by_name": None})):
            module = types.ModuleType(name)
            for key, value in attrs.items():
                setattr(module, key, value)
            sys.modules.setdefault(name, module)
        from room_m.utils import items

        with mock.patch.object(items, "phase_by_name", lambda doc, name: object()), \
                mock.patch.object(items, "get_item_facts", lambda doc, phase, categories, identify_by: found):
            return items.item_facts(None, "New Construction", [])

    def test_both_are_passed_through_as_strings(self):
        facts = self.run_with({101: {"reference": 9, "category": "Furniture", "family_name": "Bed", "host": None,
                                     "placement_type": "WorkPlaneBased", "unique_id": "u-101"}})
        self.assertEqual("WorkPlaneBased", facts["101"]["placement_type"])
        self.assertEqual("u-101", facts["101"]["unique_id"])

    def test_a_duhast_without_them_gives_none_not_a_key_error(self):
        facts = self.run_with({101: {"reference": 9, "category": "Furniture", "family_name": "Bed", "host": None}})
        self.assertIsNone(facts["101"]["placement_type"])
        self.assertIsNone(facts["101"]["unique_id"])


if __name__ == "__main__":
    unittest.main()

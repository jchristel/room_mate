"""Planning a SET task (`room_m.rls_set_plan`, multi-room RLS). Stdlib unittest, CPython, no Revit."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import rls_set_plan as sp  # noqa: E402
from room_m import rls_task as rt  # noqa: E402

SETTINGS = {
    "sheet_sizes": {"a1": "TB : A1"},
    "plan_view_type": "RLS Plan", "elevation_view_type": "RLS Elevation", "three_d_view_type": "RLS 3D", "schedule_base": "Room Base",
    "room_tag": "Room Tag : Std", "independent_tag": "Multi : Std",
    "sheet_number": "RLS-{Number}", "sheet_name": "{Name} layout",
    "view_name_plan": "{Number} PLAN", "view_name_elevation": "{Number} ELEV {Seq_Counter}",
    "view_name_three_d": "{Number} 3D", "view_name_schedule": "{Number} SCHEDULE",
    "set_sheet_number": "SET-{Number}-{Sheet_Counter}", "set_sheet_name": "{Rooms:Name}",
    "set_view_name_plan": "{Rooms:Number} PLAN",
}

ROOMS = {
    "1": {"Number": "00.02", "Name": "OFFICE"},
    "2": {"Number": "00.03", "Name": "WC"},
}


def elevation(i):
    return {"marker_index": i, "line_start": [0, 0], "line_end": [1000, 0], "looks": [0, 1], "depth_mm": 2000.0,
            "height_mm": 2400.0, "scale": 25.0}


def task(sheets=None, **set_changes):
    """OFFICE (lead) and WC: the plan and OFFICE's views on sheet 1, WC's on sheet 2, no 3D."""
    rls_set = {
        "sheet_size": "a1", "paper_mm": [841.0, 594.0], "lead_room": "1",
        "rooms": [{"room_id": "1", "number": "00.02", "name": "OFFICE 00.02"}, {"room_id": "2", "number": "00.03", "name": "WC 00.03"}],
        "plan": {"crop": {"min": [0, 0], "max": [5000, 4000], "offsets_mm": {}, "turn": {"angle_deg": 30.0, "by": "room_angle"}}, "scale": 50.0},
        "markers": [
            {"room_id": "1", "location": [2000.0, 2000.0], "views": [elevation(2), elevation(0)]},
            {"room_id": "2", "location": [4000.0, 2000.0], "views": [elevation(4)]},
        ],
        "schedules": ["1", "2"],
        "three_d": False,
        "sheets": sheets if sheets is not None else [
            {"views": [
                {"kind": "plan", "centre_mm": [100.0, 400.0], "size_mm": [100.0, 80.0]},
                {"kind": "elevation", "room_id": "1", "marker_index": 0, "centre_mm": [300.0, 400.0], "size_mm": [40.0, 96.0]},
                {"kind": "elevation", "room_id": "1", "marker_index": 2, "centre_mm": [400.0, 400.0], "size_mm": [40.0, 96.0]},
                {"kind": "schedule", "room_id": "1", "centre_mm": [600.0, 400.0], "size_mm": [133.0, 65.0]},
            ]},
            {"views": [
                {"kind": "elevation", "room_id": "2", "marker_index": 4, "centre_mm": [300.0, 300.0], "size_mm": [40.0, 96.0]},
                {"kind": "schedule", "room_id": "2", "centre_mm": [600.0, 300.0], "size_mm": [133.0, 65.0]},
            ]},
        ],
    }
    rls_set.update(set_changes)
    return {
        "id": "t9", "kind": "create_rls_set", "phase": "(the room's own phase)",
        "room": {"room_id": "1", "number": "00.02", "name": "OFFICE 00.02"},
        "sheets": [{"sheet_number": "50H-00.02", "sheet": {"view_ports": [
            {"vp_type": "floor plan", "view_id": 10, "view": {"view_scale": 50}},
            {"vp_type": "elevation", "view_id": 11, "view": {"view_scale": 25}},
            {"vp_type": "three d", "view_id": 12, "view": {
                "forward_direction": {"x": 1, "y": 0, "z": -0.5}, "up_direction": {"x": 0, "y": 0, "z": 1}}},
        ]}}],
        "rls_set": rls_set,
        "findings": [],
    }


class Facts(object):
    def __init__(self, **over):
        self.over = over

    def get(self, key, default):
        return self.over.get(key, default)

    def room_phase_name(self, room):
        return None if room in self.get("no_phase", []) else "New Construction"

    def find_room(self, number, room_id):
        return None if room_id in self.get("gone", []) else room_id

    def room_values(self, room):
        return self.get("values", ROOMS)[room]

    def title_block_exists(self, name):
        return True

    def view_type_exists(self, kind, name):
        return True

    def schedule_exists(self, name):
        return True

    def schedule_filters_on_room_number(self, name):
        return True

    def tag_type_exists(self, kind, name):
        return True

    def sheet_exists(self, number):
        return number in self.get("sheets", [])

    def views_in_use(self, names):
        return [n for n in names if n in self.get("views", [])]

    def line_detail_exists(self, name):
        return True


def verdict(t=None, settings=None, facts=None):
    return rt.plan_tasks([t or task()], settings or SETTINGS, facts or Facts())[0]


def codes(findings):
    return [f["code"] for f in findings]


class Naming(unittest.TestCase):
    def test_rooms_of_a_parameter_lists_every_room_once_lead_first(self):
        every = [{"Number": "01.06", "Level": "L1"}, {"Number": "01.07", "Level": "L1"}]
        self.assertEqual(("01.06, 01.07 on L1", []), sp.render_set("{Rooms:Number} on {Rooms:Level}", every[0], every))
        self.assertEqual(("SET-01.06-2", []), sp.render_set("SET-{Number}-{Sheet_Counter}", every[0], every, {"Sheet_Counter": 2}))
        text, missing = sp.render_set("{Rooms:Wing}", every[0], [every[0], {"Number": "01.07", "Wing": "B"}])
        self.assertEqual(["Rooms:Wing"], missing, "one room without it is enough to abort")

    def test_the_sheets_the_plan_the_elevations_and_the_schedules_get_their_names(self):
        v = verdict()
        self.assertEqual([], v["aborts"], v["aborts"])
        plan = v["plan"]
        self.assertEqual(["SET-00.02-1", "SET-00.02-2"], [s["number"] for s in plan["sheets"]], "the lead's number, the sheet's place")
        self.assertEqual("OFFICE, WC", plan["sheets"][0]["name"])
        self.assertEqual("00.02, 00.03 PLAN", plan["plan_view"]["name"])
        self.assertEqual(
            ["00.02 ELEV 1", "00.02 ELEV 2", "00.03 ELEV 1"], [e["name"] for e in plan["elevations"]],
            "each room's own number, counted 1 to 4 within the room, in marker-index order")
        self.assertEqual(["00.02 SCHEDULE", "00.03 SCHEDULE"], [s["name"] for s in plan["schedules"]])

    def test_an_empty_set_template_falls_back_to_the_single_room_one(self):
        settings = dict(SETTINGS, set_view_name_plan="", set_sheet_name="")
        plan = verdict(settings=settings)["plan"]
        self.assertEqual("00.02 PLAN", plan["plan_view"]["name"])
        self.assertEqual("OFFICE layout", plan["sheets"][1]["name"])

    def test_more_than_one_sheet_needs_the_sheet_counter_in_the_number(self):
        settings = dict(SETTINGS, set_sheet_number="SET-{Number}")
        self.assertIn(rt.TEMPLATE_UNRESOLVED, codes(verdict(settings=settings)["aborts"]))
        one_sheet = task(sheets=[{"views": [{"kind": "plan", "centre_mm": [100.0, 100.0], "size_mm": [10.0, 10.0]}]}],
                         markers=[], schedules=[])
        self.assertEqual([], verdict(one_sheet, settings)["aborts"], "one sheet needs no counter")

    def test_a_room_without_a_parameter_the_set_names_aborts_and_says_which_template(self):
        values = {"1": ROOMS["1"], "2": {"Number": "00.03"}}
        aborts = verdict(facts=Facts(values=values))["aborts"]
        self.assertEqual([rt.TEMPLATE_UNRESOLVED], codes(aborts))
        self.assertIn("{Rooms:Name}", aborts[0]["message"])


class Placing(unittest.TestCase):
    def test_every_view_knows_its_sheet_and_centre(self):
        plan = verdict()["plan"]
        self.assertEqual((0, [100.0, 400.0]), (plan["plan_view"]["sheet"], plan["plan_view"]["centre_mm"]))
        wc = [e for e in plan["elevations"] if e["room_id"] == "2"][0]
        self.assertEqual((1, [300.0, 300.0]), (wc["sheet"], wc["centre_mm"]))
        self.assertEqual("2", wc["marker_id"], "one marker per room")
        self.assertEqual([600.0, 300.0], plan["schedules"][1]["centre_mm"])
        self.assertEqual([133.0, 65.0], plan["schedules"][1]["size_mm"], "the schedule is placed by its corner")

    def test_a_schedule_on_several_sheets_is_one_schedule_placed_on_each(self):
        t = task()
        t["rls_set"]["sheets"][1]["views"].append(
            {"kind": "schedule", "room_id": "1", "centre_mm": [600.0, 150.0], "size_mm": [133.0, 65.0]})
        office = verdict(t)["plan"]["schedules"][0]
        self.assertEqual("00.02 SCHEDULE", office["name"], "one view, one name")
        self.assertEqual([(0, [600.0, 400.0]), (1, [600.0, 150.0])], [(p["sheet"], p["centre_mm"]) for p in office["places"]])

    def test_detail_numbers_run_per_sheet(self):
        plan = verdict()["plan"]
        self.assertEqual(2, len(plan["sheets"][0]["detail_numbers"]["elevation"]))
        self.assertEqual(1, len(plan["sheets"][0]["detail_numbers"]["plan"]))
        self.assertEqual(["1"], [str(n) for n in plan["sheets"][1]["detail_numbers"]["elevation"]])
        self.assertEqual([], plan["sheets"][1]["detail_numbers"]["plan"])

    def test_the_3d_view_turns_with_the_plan_and_keeps_the_walls_between_the_rooms(self):
        t = task(three_d=True)
        t["rls_set"]["sheets"][0]["views"].append({"kind": "three_d", "centre_mm": [700.0, 150.0], "size_mm": [150.0, 150.0]})
        plan = verdict(t, dict(SETTINGS, set_view_name_three_d="{Number} SET 3D"))["plan"]
        (view,) = plan["three_ds"]
        self.assertEqual("00.02 SET 3D", view["name"])
        self.assertAlmostEqual(0.866, view["forward"][0], 3, "turned 30 degrees with the plan")
        self.assertTrue(view["keep_walls"])
        self.assertEqual((0, [700.0, 150.0]), (view["sheet"], view["centre_mm"]))


class Checks(unittest.TestCase):
    def test_a_room_gone_or_without_a_phase_aborts_and_names_it(self):
        self.assertEqual([rt.ROOM_MISSING], codes(verdict(facts=Facts(gone=["2"]))["aborts"]))
        aborts = verdict(facts=Facts(no_phase=["2"]))["aborts"]
        self.assertEqual([rt.PHASE_MISSING], codes(aborts))
        self.assertIn("00.03", aborts[0]["message"])

    def test_a_sheet_or_view_name_taken_aborts_and_two_sets_of_one_list_cannot_collide(self):
        self.assertIn(rt.SHEET_EXISTS, codes(verdict(facts=Facts(sheets=["SET-00.02-2"]))["aborts"]))
        self.assertIn(rt.VIEW_EXISTS, codes(verdict(facts=Facts(views=["00.03 ELEV 1"]))["aborts"]))
        both = rt.plan_tasks([task(), task()], SETTINGS, Facts())
        self.assertEqual([], both[0]["aborts"])
        self.assertIn(rt.SHEET_EXISTS, codes(both[1]["aborts"]))

    def test_two_views_of_the_set_given_one_name_abort(self):
        settings = dict(SETTINGS, view_name_elevation="ELEV {Seq_Counter}")
        self.assertIn(rt.VIEW_EXISTS, codes(verdict(settings=settings)["aborts"]), "two rooms' ELEV 1")

    def test_a_set_without_its_plan_warns_that_its_elevations_cannot_be_made(self):
        v = verdict(task(plan=None))
        self.assertEqual([], v["aborts"])
        self.assertIn(sp.NO_PLAN_FOR_ELEVATIONS, codes(v["warnings"]))

    def test_a_single_room_task_still_goes_to_the_single_room_planner(self):
        single = {"id": "t1", "kind": "create_rls", "room": {"room_id": "1", "number": "00.02"}, "sheets": [], "rls": {}}
        v = rt.plan_tasks([single], SETTINGS, Facts())[0]
        self.assertNotIn("set", v["plan"] or {})


if __name__ == "__main__":
    unittest.main()

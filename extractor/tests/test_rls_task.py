"""Planning a room layout sheet task (`room_m.rls_task`). Stdlib unittest, CPython, no Revit."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import rls_task as rt  # noqa: E402

SETTINGS = {
    "sheet_sizes": {"a0": "TB : A0", "a1": "TB : A1", "a2": "TB : A2", "a3": "TB : A3"},
    "plan_view_type": "RLS Plan", "elevation_view_type": "RLS Elevation", "schedule_base": "Room Base",
    "room_tag": "Room Tag : Std", "independent_tag": "Multi : Std",
    "sheet_number": "RLS-{Number}", "sheet_name": "{Name} layout", "view_name": "{Number} {Name}",
}


def viewport(vp_type, view_id, x, y, scale=None, box=None):
    return {"vp_type": vp_type, "view_id": view_id, "centre_point": {"x": x, "y": y},
            "bounding_box": box or {"min_x": x - 100, "max_x": x + 100, "min_y": y - 100, "max_y": y + 100},
            "view": {"view_scale": scale} if scale else {}}


def task(**changes):
    t = {
        "id": "t1", "kind": "create_rls", "phase": "New Construction", "room": {"room_id": "9", "number": "01.09", "name": "STUDY"},
        "sheets": [{"sheet_number": "A5", "sheet": {"view_ports": [
            viewport("floor plan", 10, 300, 200, 50), viewport("elevation", 1, 100, 100, 50),
            viewport("elevation", 2, 500, 100, 50), viewport("schedule", 3, 400, 50)]}}],
        "rls": {"sheet_size": "a1", "markers": [{"marker_id": 7, "location": [1000.0, 2000.0], "views": [
            {"view_id": 2, "marker_index": 1, "line_start": [0, 0], "line_end": [2000, 0], "looks": [1, 0], "depth_mm": 2500.0},
            {"view_id": 1, "marker_index": 0, "line_start": [0, 0], "line_end": [2000, 0], "looks": [0, -1], "depth_mm": 2000.0}]}],
            "tags": {"views": [{"view_id": 10, "view_type": "floor plan", "tagged": [
                {"item_id": "55", "source": "rule", "head": [100.0, 200.0]}, {"item_id": "56", "source": "choice"}]}]}},
        "findings": [],
    }
    t.update(changes)
    return t


class Facts(object):
    def __init__(self, **over):
        self.over = over

    def get(self, key, default):
        return self.over.get(key, default)

    def phase_exists(self, name):
        return self.get("phase", True)

    def find_room(self, number, room_id):
        return self.get("room", object())

    def room_values(self, room):
        return self.get("values", {"Number": "01.09", "Name": "STUDY"})

    def title_block_exists(self, name):
        return self.get("title_block", True)

    def view_type_exists(self, kind, name):
        return self.get("view_type", True)

    def schedule_exists(self, name):
        return self.get("schedule", True)

    def tag_type_exists(self, kind, name):
        return self.get("tag_" + kind, True)

    def sheet_exists(self, number):
        return self.get("sheet", False)

    def views_in_use(self, names):
        return self.get("views", [])


def verdict(t=None, settings=None, facts=None):
    return rt.plan_tasks([t or task()], settings or SETTINGS, facts or Facts())[0]


def codes(findings):
    return [f["code"] for f in findings]


class Render(unittest.TestCase):
    def test_tokens_are_filled_from_the_room_ignoring_case(self):
        self.assertEqual(("RLS-01.09 STUDY", []), rt.render("RLS-{number} {NAME}", {"Number": "01.09", "Name": "STUDY"}))

    def test_a_token_the_room_does_not_have_is_reported_not_blanked(self):
        text, missing = rt.render("{Number}-{Wing}", {"Number": "1"})
        self.assertEqual(["Wing"], missing)

    def test_characters_revit_refuses_in_a_number_become_dashes(self):
        self.assertEqual("RLS-1-2-3", rt.clean_number("RLS-1/2:3"))


class Plan(unittest.TestCase):
    def test_a_good_task_has_no_aborts_and_no_warnings(self):
        v = verdict()
        self.assertEqual([], v["aborts"], v["aborts"])
        self.assertEqual([], v["warnings"])
        p = v["plan"]
        self.assertEqual("RLS-01.09", p["number"])
        self.assertEqual("STUDY layout", p["name"])
        self.assertEqual("TB : A1", p["title_block"], "the size the TASK asked for, not the template's")
        self.assertEqual("01.09 STUDY PLAN", p["plan_name"])
        self.assertEqual(50, p["plan_scale"])

    def test_elevations_are_named_marker_by_marker_in_index_order_and_find_their_template_viewport(self):
        p = verdict()["plan"]
        self.assertEqual(["01.09 STUDY ELEV 1", "01.09 STUDY ELEV 2"], [e["name"] for e in p["elevations"]])
        self.assertEqual([1, 2], [e["view_id"] for e in p["elevations"]], "index 0 first")
        self.assertEqual(100, p["elevations"][0]["viewport"]["centre_point"]["x"])
        self.assertEqual(50, p["elevations"][0]["scale"])

    def test_the_schedule_and_the_plan_tags_come_from_the_template_and_the_server(self):
        p = verdict()["plan"]
        self.assertEqual("01.09 STUDY SCHEDULE", p["schedule_name"])
        self.assertEqual(3, p["schedule_viewport"]["view_id"])
        self.assertEqual(["55"], [t["item_id"] for t in p["tags"]], "only tags with a head are placed")

    def test_a_task_with_no_size_falls_back_to_the_templates_and_then_aborts(self):
        t = task()
        del t["rls"]["sheet_size"]
        self.assertEqual([rt.NO_SIZE], codes(verdict(t)["aborts"]))
        t["sheets"][0]["sheet"]["sheet_size"] = "A2"
        v = verdict(t)
        self.assertEqual("TB : A2", v["plan"]["title_block"])


class UnhappyPath(unittest.TestCase):
    def aborts(self, **facts):
        return codes(verdict(facts=Facts(**facts))["aborts"])

    def test_the_sheet_already_exists_aborts(self):
        self.assertEqual([rt.SHEET_EXISTS], self.aborts(sheet=True))

    def test_a_view_already_existing_aborts_and_names_it(self):
        v = verdict(facts=Facts(views=["01.09 STUDY PLAN"]))
        self.assertEqual([rt.VIEW_EXISTS], codes(v["aborts"]))
        self.assertIn("01.09 STUDY PLAN", v["aborts"][0]["message"])

    def test_no_title_block_family_for_the_size_aborts(self):
        self.assertEqual([rt.NO_TITLE_BLOCK], self.aborts(title_block=False))

    def test_a_view_type_or_the_schedule_missing_from_the_model_aborts(self):
        self.assertEqual([rt.VIEW_TYPE_MISSING, rt.VIEW_TYPE_MISSING], self.aborts(view_type=False))
        self.assertEqual([rt.SCHEDULE_MISSING], self.aborts(schedule=False))

    def test_a_template_the_room_cannot_fill_aborts(self):
        v = verdict(facts=Facts(values={"Number": "01.09"}))
        self.assertIn(rt.TEMPLATE_UNRESOLVED, codes(v["aborts"]))

    def test_the_room_and_the_phase_missing_abort(self):
        self.assertEqual([rt.ROOM_MISSING], self.aborts(room=None))
        self.assertIn(rt.PHASE_MISSING, self.aborts(phase=False))

    def test_incomplete_settings_abort_and_name_what_is_missing(self):
        v = verdict(settings=dict(SETTINGS, schedule_base=""))
        self.assertEqual([rt.SETTINGS_INCOMPLETE], codes(v["aborts"]))
        self.assertIn("schedule base", v["aborts"][0]["message"])

    def test_another_kind_of_task_is_not_this_scripts(self):
        v = verdict(t=task(kind="populate_room"))
        self.assertEqual([rt.UNKNOWN_KIND], codes(v["aborts"]))

    def test_two_tasks_of_one_list_cannot_make_the_same_sheet(self):
        a, b = task(id="t1"), task(id="t2")
        first, second = rt.plan_tasks([a, b], SETTINGS, Facts())
        self.assertEqual([], first["aborts"])
        self.assertEqual([rt.SHEET_EXISTS, rt.VIEW_EXISTS], codes(second["aborts"]))


class Warnings(unittest.TestCase):
    def test_no_room_tag_or_independent_tag_continues_with_a_warning(self):
        v = verdict(facts=Facts(tag_room=False, tag_independent=False))
        self.assertEqual([], v["aborts"])
        self.assertEqual([rt.NO_ROOM_TAG, rt.NO_INDEPENDENT_TAG], codes(v["warnings"]))

    def test_tags_not_set_in_the_settings_warn_too(self):
        v = verdict(settings=dict(SETTINGS, room_tag="", independent_tag=""))
        self.assertEqual([rt.NO_ROOM_TAG, rt.NO_INDEPENDENT_TAG], codes(v["warnings"]))

    def test_elevations_the_server_could_not_place_warn(self):
        v = verdict(t=task(findings=[{"kind": "elevation_not_placed", "message": "different edge counts"}]))
        self.assertEqual([rt.ELEVATION_NOT_PLACED], codes(v["warnings"]))


class Source(unittest.TestCase):
    def test_the_apply_button_leaves_the_rls_tasks_alone(self):
        with open(os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m", "room_mate.py")) as f:
            text = f.read()
        self.assertIn('kinds=["populate_room", "update_room"]', text)
        self.assertIn('kinds=["create_rls"]', text)


if __name__ == "__main__":
    unittest.main()

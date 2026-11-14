"""Planning a room layout sheet task (`room_m.rls_task`). Stdlib unittest, CPython, no Revit."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import rls_task as rt  # noqa: E402

SETTINGS = {
    "sheet_sizes": {"a0": "TB : A0", "a1": "TB : A1", "a2": "TB : A2", "a3": "TB : A3"},
    "plan_view_type": "RLS Plan", "elevation_view_type": "RLS Elevation", "three_d_view_type": "RLS 3D", "schedule_base": "Room Base",
    "room_tag": "Room Tag : Std", "independent_tag": "Multi : Std",
    "sheet_number": "RLS-{Number}", "sheet_name": "{Name} layout", "view_name": "{Number} {Name}",
}


def viewport(vp_type, view_id, x, y, scale=None, box=None, crop=None):
    return {"vp_type": vp_type, "view_id": view_id, "centre_point": {"x": x, "y": y},
            "bounding_box": box or {"min_x": x - 100, "max_x": x + 100, "min_y": y - 100, "max_y": y + 100},
            "view": dict({"view_scale": scale} if scale else {}, **({"bounding_box": crop} if crop else {}))}


def task(**changes):
    t = {
        "id": "t1", "kind": "create_rls", "phase": "New Construction", "room": {"room_id": "9", "number": "01.09", "name": "STUDY"},
        "sheets": [{"sheet_number": "A5", "sheet": {"view_ports": [
            viewport("floor plan", 10, 300, 200, 50), viewport("elevation", 1, 100, 100, 50, crop={"min_x": 0, "max_x": 5000, "min_y": 3395, "max_y": 6695}),
            viewport("elevation", 2, 500, 100, 50), viewport("schedule", 3, 400, 50)]}}],
        "rls": {"sheet_size": "a1", "markers": [{"marker_id": 7, "location": [1000.0, 2000.0], "views": [
            {"view_id": 2, "marker_index": 1, "line_start": [0, 0], "line_end": [2000, 0], "looks": [1, 0], "depth_mm": 2500.0},
            {"view_id": 1, "marker_index": 0, "line_start": [0, 0], "line_end": [2000, 0], "looks": [0, -1], "depth_mm": 2000.0}]}],
            "tags": {"views": [{"view_id": 10, "view_type": "floor plan", "tagged": [
                {"item_id": "55", "source": "rule", "head": [100.0, 200.0]}, {"item_id": "56", "source": "choice"}]},
                {"view_id": 1, "view_type": "elevation", "tagged": [
                    {"item_id": "57", "source": "rule", "view_offset_mm": [400.0, 1800.0], "along_line_mm": 500.0},
                    {"item_id": "58", "source": "choice", "along_line_mm": 900.0}]}]}},
        "findings": [],
    }
    t.update(changes)
    return t


class Facts(object):
    def __init__(self, **over):
        self.over = over

    def get(self, key, default):
        return self.over.get(key, default)

    def room_phase_name(self, room):
        return "New Construction" if self.get("phase", True) else None

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

    def schedule_filters_on_room_number(self, name):
        return self.get("schedule_room_filter", True)

    def tag_type_exists(self, kind, name):
        return self.get("tag_" + kind, True)

    def sheet_exists(self, number):
        return self.get("sheet", False)

    def views_in_use(self, names):
        return self.get("views", [])

    def line_detail_exists(self, name):
        return self.get("line_detail", True)


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
        self.assertEqual(["01.09 STUDY ELEV 1", "01.09 STUDY ELEV 2"], [e["name"] for e in p["elevations"]],
                         "the old single view name, read per kind")
        self.assertEqual([1, 2], [e["view_id"] for e in p["elevations"]], "index 0 first")
        self.assertEqual(100, p["elevations"][0]["viewport"]["centre_point"]["x"])
        self.assertEqual(50, p["elevations"][0]["scale"])
        self.assertEqual(3300, p["elevations"][0]["height_mm"], "cropped to the height the template's view had")

    def test_every_viewport_kind_gets_its_detail_numbers_from_the_settings(self):
        d = verdict()["plan"]["detail_numbers"]
        self.assertEqual(["1", "2"], d["elevation"])
        self.assertEqual(["3"], d["plan"], "same style as the elevations: one run")

    def test_the_schedule_and_the_plan_tags_come_from_the_template_and_the_server(self):
        p = verdict()["plan"]
        self.assertEqual("01.09 STUDY SCHEDULE", p["schedule_name"])
        self.assertEqual(3, p["schedule_viewport"]["view_id"])
        self.assertEqual(["55"], [t["item_id"] for t in p["tags"]], "only tags with a head are placed")

    def test_elevation_tags_come_by_template_view_and_only_those_with_an_offset(self):
        p = verdict()["plan"]
        self.assertEqual({1: ["57"]}, dict((k, [t["item_id"] for t in v]) for k, v in p["elevation_tags"].items()))
        self.assertEqual(["55"], [t["item_id"] for t in p["tags"]], "the plan's tags are not the elevations'")

    def test_a_base_schedule_not_filtered_first_on_the_room_number_aborts(self):
        v = verdict(facts=Facts(schedule_room_filter=False))
        self.assertEqual([rt.SCHEDULE_NOT_ROOM_FILTERED], codes(v["aborts"]))

    def test_a_missing_base_schedule_is_only_missing_not_also_unfiltered(self):
        v = verdict(facts=Facts(schedule=False, schedule_room_filter=False))
        self.assertEqual([rt.SCHEDULE_MISSING], codes(v["aborts"]))

    def test_a_task_with_no_size_falls_back_to_the_templates_and_then_aborts(self):
        t = task()
        del t["rls"]["sheet_size"]
        self.assertEqual([rt.NO_SIZE], codes(verdict(t)["aborts"]))
        t["sheets"][0]["sheet"]["sheet_size"] = "A2"
        v = verdict(t)
        self.assertEqual("TB : A2", v["plan"]["title_block"])


class TitleOnSheet(unittest.TestCase):
    TITLES = dict(SETTINGS, title_on_sheet_plan="PLAN - {Name}", title_on_sheet_elevation="ELEVATION {Seq_Counter} {Number}",
                  title_on_sheet_three_d="3D {seq_counter}", title_on_sheet_schedule="SCHEDULE {Seq_Counter}")

    def test_each_kind_takes_its_template_and_seq_counter_counts_viewports_of_that_kind_from_1(self):
        t = task()
        t["sheets"][0]["sheet"]["view_ports"].append(dict(viewport("three d", 4, 600, 100),
            view={"forward_direction": {"x": 1, "y": 1, "z": -1}, "up_direction": {"x": 0, "y": 0, "z": 1}}))
        v = verdict(t, settings=self.TITLES)
        self.assertEqual([], v["aborts"], v["aborts"])
        p = v["plan"]
        self.assertEqual("PLAN - STUDY", p["plan_title"])
        self.assertEqual(["ELEVATION 1 01.09", "ELEVATION 2 01.09"], [e["title"] for e in p["elevations"]])
        self.assertEqual(["3D 1"], [x["title"] for x in p["three_ds"]], "case is ignored, as for room parameters")
        self.assertEqual("SCHEDULE 1", p["schedule_title"])

    def test_no_template_leaves_revits_own_title(self):
        p = verdict()["plan"]
        self.assertIsNone(p["plan_title"])
        self.assertIsNone(p["schedule_title"])
        self.assertEqual([None, None], [e["title"] for e in p["elevations"]])

    def test_a_room_parameter_the_room_lacks_aborts_once_per_kind(self):
        v = verdict(settings=dict(SETTINGS, title_on_sheet_elevation="{Seq_Counter} {Wing}"))
        self.assertEqual([rt.TEMPLATE_UNRESOLVED], codes(v["aborts"]))
        self.assertIn("{Wing}", v["aborts"][0]["message"])


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


def with_three_d(forward, up, t=None):
    t = t or task()
    vp = viewport("three d", 99, 400, 300)
    vp["view"] = {"forward_direction": forward, "up_direction": up}
    t["sheets"][0]["sheet"]["view_ports"].append(vp)
    return t


class ElevationGrid(unittest.TestCase):
    def test_the_plan_carries_the_grid_family_and_none_when_not_set(self):
        self.assertIsNone(verdict()["plan"]["elevation_grid"])
        v = verdict(settings=dict(SETTINGS, elevation_grid="Grid_ANN : Setout"))
        self.assertEqual("Grid_ANN : Setout", v["plan"]["elevation_grid"])
        self.assertEqual(([], []), (v["aborts"], v["warnings"]))

    def test_a_grid_family_the_model_lacks_only_warns(self):
        v = verdict(settings=dict(SETTINGS, elevation_grid="Gone : X"), facts=Facts(line_detail=False))
        self.assertEqual([], v["aborts"])
        self.assertEqual([rt.NO_ELEVATION_GRID], codes(v["warnings"]))
        self.assertIn("'Gone : X'", v["warnings"][0]["message"])


class ViewNames(unittest.TestCase):
    PER_KIND = dict(SETTINGS, view_name_plan="P-{Number}", view_name_elevation="E-{Number}-{Seq_Counter}",
                    view_name_three_d="3D-{Number}", view_name_schedule="S-{Number}")

    def test_each_kind_takes_its_own_view_name_logic(self):
        s = dict(self.PER_KIND)
        del s["view_name"]
        v = verdict(with_three_d(ThreeD.D, ThreeD.U), settings=s)
        self.assertEqual([], v["aborts"], v["aborts"])
        p = v["plan"]
        self.assertEqual("P-01.09", p["plan_name"])
        self.assertEqual(["E-01.09-1", "E-01.09-2"], [e["name"] for e in p["elevations"]])
        self.assertEqual(["3D-01.09"], [d["name"] for d in p["three_ds"]])
        self.assertEqual("S-01.09", p["schedule_name"])

    def test_a_template_with_a_3d_view_needs_the_3d_view_name(self):
        s = dict(self.PER_KIND, view_name_three_d="")
        del s["view_name"]
        v = verdict(with_three_d(ThreeD.D, ThreeD.U), settings=s)
        self.assertEqual([rt.TEMPLATE_UNRESOLVED], codes(v["aborts"]))
        self.assertIn("3D view name logic", v["aborts"][0]["message"])
        self.assertEqual([], verdict(settings=s)["aborts"], "no 3D view on the template: no 3D name asked for")

    def test_two_views_given_one_name_abort(self):
        s = dict(self.PER_KIND, view_name_elevation="E-{Number}")
        v = verdict(settings=s)
        self.assertEqual([rt.VIEW_EXISTS], codes(v["aborts"]))
        self.assertIn("'E-01.09'", v["aborts"][0]["message"])


class ThreeD(unittest.TestCase):
    D = {"x": 203.2, "y": 203.2, "z": -101.6}
    U = {"x": 71.8, "y": 71.8, "z": 287.4}

    def test_each_3d_viewport_of_the_template_gives_a_new_view_with_its_direction(self):
        v = verdict(with_three_d(self.D, self.U))
        views = v["plan"]["three_ds"]
        self.assertEqual(1, len(views))
        self.assertEqual("01.09 STUDY 3D", views[0]["name"])
        self.assertEqual([203.2, 203.2, -101.6], views[0]["forward"])
        self.assertEqual([], v["aborts"])

    def test_the_view_name_is_taken_into_account_for_name_collisions(self):
        facts = Facts()
        facts.views_in_use = lambda names: [n for n in names if n.endswith(" 3D")]
        v = verdict(with_three_d(self.D, self.U), facts=facts)
        self.assertIn(rt.VIEW_EXISTS, codes(v["aborts"]))

    def test_a_turned_plan_turns_the_3d_direction_and_up_with_it(self):
        t = with_three_d(self.D, self.U)
        t["rls"]["plan_crop"] = {"min": [0, 0], "max": [4000, 3000], "turn": {"angle_deg": 90.0, "by": "walls"}}
        view = verdict(t)["plan"]["three_ds"][0]
        for got, want in zip(view["forward"], [-203.2, 203.2, -101.6]):
            self.assertAlmostEqual(want, got)
        for got, want in zip(view["up"], [-71.8, 71.8, 287.4]):
            self.assertAlmostEqual(want, got)

    def test_a_3d_viewport_without_a_direction_warns_and_makes_no_view(self):
        v = verdict(with_three_d({"x": 0, "y": 0, "z": 0}, self.U))
        self.assertEqual([], v["plan"]["three_ds"])
        self.assertIn(rt.THREE_D_NO_DIRECTION, codes(v["warnings"]))

    def test_a_3d_view_needs_a_chosen_3d_view_type_and_a_task_without_one_does_not(self):
        no_type = dict(SETTINGS, three_d_view_type=None)
        self.assertIn(rt.VIEW_TYPE_MISSING, codes(verdict(with_three_d(self.D, self.U), settings=no_type)["aborts"]))
        self.assertEqual([], verdict(settings=no_type)["aborts"], "no 3D view on the template: no type asked for")

    def test_a_3d_view_type_the_model_lacks_aborts(self):
        facts = Facts()
        facts.view_type_exists = lambda kind, name: kind != "three_d"
        self.assertIn(rt.VIEW_TYPE_MISSING, codes(verdict(with_three_d(self.D, self.U), facts=facts)["aborts"]))

    def test_the_viewport_type_is_optional_but_must_exist_when_chosen(self):
        self.assertIsNone(verdict()["plan"]["viewport_type"])
        chosen = dict(SETTINGS, viewport_type="Title w Line")
        self.assertEqual("Title w Line", verdict(settings=chosen)["plan"]["viewport_type"])
        facts = Facts()
        facts.view_type_exists = lambda kind, name: kind != "viewport"
        self.assertIn(rt.VIEW_TYPE_MISSING, codes(verdict(settings=chosen, facts=facts)["aborts"]))
        self.assertEqual([], verdict(facts=facts)["aborts"], "none chosen: nothing to check")

    def test_a_template_with_no_3d_viewport_makes_none(self):
        self.assertEqual([], verdict()["plan"]["three_ds"])


class RoomTag(unittest.TestCase):
    def test_a_head_the_server_worked_out_is_in_the_plan(self):
        t = task()
        t["rls"]["room_tag"] = {"head": [1200, 3400.5], "rule_id": "tr-1"}
        self.assertEqual([1200.0, 3400.5], verdict(t)["plan"]["room_tag_head"])

    def test_no_head_means_the_plug_in_applies_the_rules_offset(self):
        self.assertIsNone(verdict()["plan"]["room_tag_head"])


class PlanCrop(unittest.TestCase):
    def test_the_server_s_crop_becomes_a_centre_and_a_size(self):
        t = task()
        t["rls"]["plan_crop"] = {"min": [1000, 2000], "max": [5000, 4000], "offsets_mm": {}}
        self.assertEqual(
            {"centre": [3000.0, 3000.0], "size": [4000.0, 2000.0], "min": [1000.0, 2000.0], "max": [5000.0, 4000.0],
             "turn_deg": 0.0},
            verdict(t)["plan"]["plan_crop"])

    def test_a_turn_rides_the_crop_and_a_broken_one_is_no_turn(self):
        t = task()
        t["rls"]["plan_crop"] = {"min": [1000, 2000], "max": [5000, 4000], "turn": {"angle_deg": -45.0, "by": "walls"}}
        self.assertEqual(-45.0, verdict(t)["plan"]["plan_crop"]["turn_deg"])
        for broken in ("thirty", {"angle_deg": "x"}, {"angle_deg": float("nan")}, {"angle_deg": 1e9}):
            t["rls"]["plan_crop"]["turn"] = broken
            self.assertEqual(0.0, verdict(t)["plan"]["plan_crop"]["turn_deg"], broken)

    def test_no_crop_or_a_broken_one_means_the_writer_crops_round_the_room(self):
        self.assertIsNone(verdict()["plan"]["plan_crop"])
        t = task()
        t["rls"]["plan_crop"] = {"min": [5000, 2000], "max": [1000, 4000]}
        self.assertIsNone(verdict(t)["plan"]["plan_crop"])


class Source(unittest.TestCase):
    def test_sheets_and_items_go_to_their_own_handlers(self):
        from room_m import task_dispatch

        self.assertEqual(task_dispatch.HANDLER_OF["create_rls"], task_dispatch.RLS)
        self.assertEqual(task_dispatch.HANDLER_OF["populate_room"], task_dispatch.APPLY)
        self.assertEqual(task_dispatch.HANDLER_OF["update_room"], task_dispatch.APPLY)


if __name__ == "__main__":
    unittest.main()

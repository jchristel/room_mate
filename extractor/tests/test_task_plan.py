"""The rules of `room_m.task_plan`, with Revit replaced by a fake. Stdlib unittest, CPython."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import task_plan as tp  # noqa: E402


class Facts(object):
    """A model with one phase, rooms by number, and what exists in it."""

    def __init__(self, phases=("New Construction",), rooms=("01.02", "01.05"), content=(), sheets=(), families=None,
                 titleblocks=True):
        self.phases = set(phases)
        self.rooms = set(rooms)
        self.content = set(content)
        self.sheets = set(sheets)
        self.families = families  # None: everything is available
        self.titleblocks = titleblocks

    def phase_exists(self, name):
        return name in self.phases

    def find_room(self, number, room_id):
        return number if number in self.rooms else None

    def room_has_content(self, room, phase):
        return room in self.content

    def rls_number(self, task):
        return "RLS-" + task["room"]["number"]

    def rls_exists(self, number):
        return number in self.sheets

    def titleblock_available(self, sheet):
        return self.titleblocks

    def family_available(self, family, type_name):
        return self.families is None or (family, type_name) in self.families


def populate(room="01.02", **extra):
    task = {
        "id": "t-" + room,
        "kind": "populate_room",
        "phase": "New Construction",
        "room": {"room_id": "9", "number": room},
        "overwrite_layout": False,
        "create_rls": False,
        "overwrite_rls": False,
        "items": [{"family_name": "Bed", "type_name": "Queen"}],
        "sheets": [],
    }
    task.update(extra)
    return task


def codes(findings):
    return [f["code"] for f in findings]


class AbortRules(unittest.TestCase):
    def test_a_clean_task_has_no_findings(self):
        (v,) = tp.plan_tasks([populate()], Facts())
        self.assertEqual(([], [], False), (v["aborts"], v["warnings"], v["skip_sheets"]))

    def test_a_missing_phase_aborts(self):
        (v,) = tp.plan_tasks([populate(phase="Future")], Facts())
        self.assertIn(tp.PHASE_MISSING, codes(v["aborts"]))

    def test_a_missing_room_aborts_and_asks_nothing_more_of_it(self):
        (v,) = tp.plan_tasks([populate(room="99.99")], Facts())
        self.assertEqual([tp.ROOM_MISSING], codes(v["aborts"]))

    def test_content_in_the_phase_aborts_unless_overwriting(self):
        facts = Facts(content=["01.02"])
        (kept,) = tp.plan_tasks([populate()], facts)
        (over,) = tp.plan_tasks([populate(overwrite_layout=True)], facts)
        self.assertEqual([tp.ROOM_HAS_CONTENT], codes(kept["aborts"]))
        self.assertEqual([], over["aborts"])

    def test_an_unknown_kind_aborts(self):
        (v,) = tp.plan_tasks([populate(kind="update_room")], Facts())
        self.assertEqual([tp.UNKNOWN_KIND], codes(v["aborts"]))


class SheetRules(unittest.TestCase):
    def sheeted(self, **extra):
        return populate(create_rls=True, sheets=[{"sheet_name": "RLS A"}], **extra)

    def test_no_sheet_family_aborts(self):
        (v,) = tp.plan_tasks([self.sheeted()], Facts(titleblocks=False))
        self.assertEqual([tp.NO_TITLEBLOCK], codes(v["aborts"]))

    def test_an_existing_rls_warns_and_skips_the_sheets_unless_overwriting(self):
        facts = Facts(sheets=["RLS-01.02"])
        (kept,) = tp.plan_tasks([self.sheeted()], facts)
        (over,) = tp.plan_tasks([self.sheeted(overwrite_rls=True)], facts)
        self.assertEqual(([], [tp.RLS_EXISTS], True), (kept["aborts"], codes(kept["warnings"]), kept["skip_sheets"]))
        self.assertEqual(([], [], False), (over["aborts"], over["warnings"], over["skip_sheets"]))

    def test_a_populate_whose_blueprint_has_no_sheets_warns_but_still_places(self):
        (v,) = tp.plan_tasks([populate(create_rls=True)], Facts())
        self.assertEqual(([], [tp.NO_SHEETS], True), (v["aborts"], codes(v["warnings"]), v["skip_sheets"]))

    def test_create_rls_with_no_sheet_layout_aborts(self):
        task = populate(kind="create_rls")
        (v,) = tp.plan_tasks([task], Facts())
        self.assertEqual([tp.NO_SHEETS], codes(v["aborts"]))

    def test_create_rls_checks_no_layout(self):
        task = populate(kind="create_rls", sheets=[{"sheet_name": "RLS A"}], items=[])
        (v,) = tp.plan_tasks([task], Facts(content=["01.02"]))
        self.assertEqual(([], []), (v["aborts"], v["warnings"]))

    def test_two_tasks_making_one_sheet_warn_on_the_second(self):
        tasks = [self.sheeted(), dict(self.sheeted(), id="second", kind="create_rls")]
        first, second = tp.plan_tasks(tasks, Facts())
        self.assertEqual([], first["warnings"])
        self.assertEqual([tp.RLS_DUPLICATE], codes(second["warnings"]))
        self.assertTrue(second["skip_sheets"])


class FamilyRules(unittest.TestCase):
    def test_a_missing_family_warns_once_per_family_and_type(self):
        items = [{"family_name": "Bed", "type_name": "Queen"}] * 3 + [{"family_name": "Desk", "type_name": "S"}]
        (v,) = tp.plan_tasks([populate(items=items)], Facts(families={("Desk", "S")}))
        self.assertEqual([tp.FAMILY_MISSING], codes(v["warnings"]))
        self.assertIn("Bed", v["warnings"][0]["message"])


class Reporting(unittest.TestCase):
    def mixed(self):
        tasks = [populate("01.02"), populate("99.99"), populate("01.05", items=[{"family_name": "X", "type_name": "Y"}])]
        return tp.plan_tasks(tasks, Facts(families=set()))

    def test_counts(self):
        # 01.02 warns too (its Bed is missing), so: 0 clear, 2 warned, 1 aborted.
        self.assertEqual((0, 2, 1), tp.summarise(self.mixed()))
        self.assertTrue(tp.has_warnings(self.mixed()))

    def test_aborts_are_reported_before_work_whatever_the_person_chooses(self):
        reports = tp.aborted_reports(self.mixed(), proceed=True)
        self.assertEqual(["t-99.99"], [r["task_id"] for r in reports])
        self.assertEqual("aborted", reports[0]["status"])

    def test_declining_the_warnings_aborts_those_tasks_with_the_reason(self):
        reports = tp.aborted_reports(self.mixed(), proceed=False)
        self.assertEqual(["t-01.02", "t-99.99", "t-01.05"], [r["task_id"] for r in reports])
        self.assertEqual("stopped at the warnings", reports[0]["messages"][0])
        self.assertEqual([], tp.runnable(self.mixed(), proceed=False))

    def test_proceeding_runs_the_warned_tasks_and_reports_them_with_warnings(self):
        run = tp.runnable(self.mixed(), proceed=True)
        self.assertEqual(["t-01.02", "t-01.05"], [v["task"]["id"] for v in run])
        self.assertEqual("completed_with_warnings", tp.result_report(run[0])["status"])

    def test_a_clean_run_completes_and_an_error_fails(self):
        (v,) = tp.plan_tasks([populate()], Facts())
        self.assertEqual("completed", tp.result_report(v)["status"])
        self.assertEqual("completed_with_warnings", tp.result_report(v, notes=["moved a door"])["status"])
        failed = tp.result_report(v, error="Revit refused")
        self.assertEqual(("failed", ["Revit refused"]), (failed["status"], failed["messages"]))


class Ascii(unittest.TestCase):
    def test_the_modules_ironpython_loads_are_ascii(self):
        here = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
        for name in ("task_plan.py", "post_tasks.py"):
            with open(os.path.join(here, name), "rb") as f:
                f.read().decode("ascii")


if __name__ == "__main__":
    unittest.main()

"""Tests for room_m.task_dispatch and the Tasks button's wiring. Run: python3 extractor/tests/test_task_dispatch.py"""
import os
import sys
import unittest

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
sys.path.insert(0, os.path.join(ROOM_M, ".."))

from room_m import task_dispatch as d  # noqa: E402


def t(tid, kind):
    return {"id": tid, "kind": kind}


class Dispatch(unittest.TestCase):
    def test_every_kind_with_a_handler_is_asked_for(self):
        self.assertEqual(d.claim_kinds(), ["create_rls", "populate_room", "update_room"])

    def test_items_are_placed_before_sheets_are_made(self):
        groups, unhandled = d.split([t("s", "create_rls"), t("p", "populate_room"), t("u", "update_room")])
        self.assertEqual([h for h, _ in groups], [d.APPLY, d.RLS])
        self.assertEqual([x["id"] for x in groups[0][1]], ["p", "u"])
        self.assertEqual(unhandled, [])

    def test_a_handler_with_no_tasks_is_left_out(self):
        groups, _ = d.split([t("s", "create_rls")])
        self.assertEqual([h for h, _ in groups], [d.RLS])

    def test_a_kind_nobody_handles_is_set_aside_and_reported_failed_by_name(self):
        groups, unhandled = d.split([t("p", "populate_room"), t("x", "paint_room")])
        self.assertEqual([x["id"] for x in unhandled], ["x"])
        reports = d.unhandled_reports(unhandled)
        self.assertEqual(reports[0]["task_id"], "x")
        self.assertEqual(reports[0]["status"], "failed")
        self.assertIn("paint_room", reports[0]["messages"][0])

    def test_the_claim_is_summed_up_by_kind(self):
        self.assertEqual(d.count_by_kind([t("a", "create_rls"), t("b", "create_rls"), t("c", "populate_room")]),
                         "2 create_rls, 1 populate_room")


class Wiring(unittest.TestCase):
    def source(self, name):
        with open(os.path.join(ROOM_M, name)) as f:
            return f.read()

    def test_every_handler_name_has_a_function_in_the_entry(self):
        text = self.source("room_mate.py")
        self.assertIn("task_dispatch.APPLY: _apply_group", text)
        self.assertIn("task_dispatch.RLS: _rls_group", text)
        for handler in set(d.HANDLER_OF.values()):
            self.assertIn(handler, d.ORDER)

    def test_there_is_one_button_for_tasks_and_the_two_old_ones_are_gone(self):
        panel = os.path.join(ROOM_M, "..", "RoomMate.extension", "duHast.tab", "RoomMate.panel")
        self.assertTrue(os.path.isdir(os.path.join(panel, "Tasks.pushbutton")))
        self.assertFalse(os.path.isdir(os.path.join(panel, "ApplyTasks.pushbutton")))
        self.assertFalse(os.path.isdir(os.path.join(panel, "RlsCreate.pushbutton")))
        with open(os.path.join(panel, "Tasks.pushbutton", "script.py")) as f:
            self.assertIn("tasks_entry(doc, uiapp, output, forms)", f.read())

    def test_the_entry_claims_every_kind_and_frees_what_it_never_reached(self):
        text = self.source("room_mate.py")
        self.assertIn("kinds=task_dispatch.claim_kinds()", text)
        self.assertIn("the session ended before this task was reached", text)


if __name__ == "__main__":
    unittest.main()

"""Tests for room_m.task_session: one run of the room tasks, shared by the Tasks button and an unattended run.
Run: python3 extractor/tests/test_task_session.py"""
import os
import sys
import unittest

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit", "room_m")
sys.path.insert(0, os.path.join(ROOM_M, ".."))

from room_m import task_dispatch as d  # noqa: E402
from room_m import task_session as s  # noqa: E402


def t(tid, kind):
    return {"id": tid, "kind": kind}


class Freer(object):
    """`free` that records what it reported."""

    def __init__(self):
        self.calls = []

    def __call__(self, tasks, status, message):
        self.calls.append(([x["id"] for x in tasks], status, message))
        return [{"task_id": x["id"], "status": status, "messages": [message]} for x in tasks]


def done(group, status="completed"):
    return [{"task_id": x["id"], "status": status} for x in group]


class Run(unittest.TestCase):
    def test_a_failed_claim_says_why_and_runs_nothing(self):
        free = Freer()
        out = s.run(lambda: (False, 503, "busy", []), {d.APPLY: lambda g: self.fail("ran")}, free)
        self.assertEqual(out["claim_error"], "503: busy")
        self.assertEqual((out["claim_status"], out["claim_text"]), (503, "busy"))
        self.assertEqual(free.calls, [])

    def test_a_claim_with_no_answer_says_so(self):
        out = s.run(lambda: (False, None, "refused", []), {}, Freer())
        self.assertEqual(out["claim_error"], "no response: refused")

    def test_nothing_queued_is_not_an_error(self):
        out = s.run(lambda: (True, 200, "[]", []), {}, Freer())
        self.assertEqual((out["claimed"], out["claim_error"], out["reports"]), ([], None, []))

    def test_items_are_placed_before_sheets_and_every_report_is_kept(self):
        order = []

        def apply(g):
            order.append(d.APPLY)
            return done(g)

        def rls(g):
            order.append(d.RLS)
            return done(g, "aborted")

        claimed = [t("s", "create_rls"), t("p", "populate_room")]
        free = Freer()
        out = s.run(lambda: (True, 200, "", claimed), {d.APPLY: apply, d.RLS: rls}, free)
        self.assertEqual(order, [d.APPLY, d.RLS])
        self.assertEqual(sorted(r["task_id"] for r in out["reports"]), ["p", "s"])
        self.assertEqual(free.calls, [])

    def test_an_unknown_kind_is_reported_failed_and_counted(self):
        free = Freer()
        out = s.run(lambda: (True, 200, "", [t("x", "paint_room")]), {}, free)
        self.assertEqual(out["unhandled"], 1)
        self.assertEqual(free.calls, [(["x"], "failed", s.UNHANDLED)])

    def test_a_handler_that_raises_has_its_tasks_freed_and_the_next_one_still_runs(self):
        def apply(g):
            raise RuntimeError("boom")

        free = Freer()
        out = s.run(lambda: (True, 200, "", [t("p", "populate_room"), t("s", "create_rls")]),
                    {d.APPLY: apply, d.RLS: done}, free)
        self.assertEqual(out["errors"], ["apply failed with exception: boom"])
        self.assertEqual(free.calls, [(["p"], "failed", s.STRANDED)])
        self.assertEqual(sorted(r["task_id"] for r in out["reports"]), ["p", "s"])

    def test_a_task_its_handler_did_not_report_is_freed(self):
        free = Freer()
        claimed = [t("a", "create_rls"), t("b", "create_rls")]
        s.run(lambda: (True, 200, "", claimed), {d.RLS: lambda g: done(g[:1])}, free)
        self.assertEqual(free.calls, [(["b"], "failed", s.STRANDED)])

    def test_a_handler_with_no_function_strands_its_group_which_is_freed(self):
        free = Freer()
        out = s.run(lambda: (True, 200, "", [t("s", "create_rls")]), {}, free)
        self.assertEqual(out["errors"], ["rls: no function runs this handler"])
        self.assertEqual(free.calls, [(["s"], "failed", s.STRANDED)])

    def test_the_log_never_fails_a_run(self):
        def log(level, line):
            raise ValueError("no disk")

        out = s.run(lambda: (True, 200, "", [t("s", "create_rls")]), {d.RLS: done}, Freer(), log=log)
        self.assertEqual([r["task_id"] for r in out["reports"]], ["s"])


class Unattended(unittest.TestCase):
    def test_warned_tasks_are_aborted_by_default_and_the_warnings_are_said(self):
        said = []
        p = s.Unattended(say=said.append)
        self.assertFalse(p.ask_proceed("sheets", ["101: no room tag"]))
        self.assertIn("aborting", said[0])
        self.assertIn("101: no room tag", said[1])

    def test_warned_tasks_run_when_asked_to(self):
        self.assertTrue(s.Unattended(proceed_with_warnings=True).ask_proceed("tasks", ["x"]))

    def test_settings_are_never_edited(self):
        said = []
        self.assertIsNone(s.Unattended(say=said.append).fix_rls_settings(None, ["not chosen: title block"]))
        self.assertIn("not chosen: title block", said[1])

    def test_a_console_that_raises_is_ignored(self):
        def say(line):
            raise IOError("closed")

        self.assertFalse(s.Unattended(say=say).ask_proceed("tasks", ["x"]))


class Wiring(unittest.TestCase):
    def source(self):
        with open(os.path.join(ROOM_M, "room_mate.py")) as f:
            return f.read()

    def test_the_button_and_the_unattended_run_share_one_session(self):
        text = self.source()
        self.assertIn("def tasks_unattended(doc, project_id, proceed_with_warnings=False, say=None, model_id=None):", text)
        self.assertEqual(text.count("= _run_session(project, "), 2)
        self.assertIn("_Asking(forms, output)", text)
        self.assertIn("task_session.Unattended(", text)

    def test_the_unattended_run_puts_the_server_reader_back(self):
        self.assertIn("server_config.set_project_reader(project_settings.server_of_active_document)",
                      self.source().split("def tasks_unattended")[1])


if __name__ == "__main__":
    unittest.main()

"""The sequence of `room_m.apply_run`, with Revit, the person and the server replaced by fakes."""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import apply_run  # noqa: E402


class Facts(object):
    def __init__(self, families=None):
        self.families = families

    def phase_exists(self, name):
        return True

    def find_room(self, number, room_id):
        return None if number == "gone" else object()

    def room_has_content(self, room, phase):
        return False

    def family_available(self, family, type_name):
        return self.families is None or (family, type_name) in self.families


def task(tid, number="1", kind="populate_room", items=()):
    return {"id": tid, "kind": kind, "phase": "P", "room": {"number": number}, "items": list(items), "sheets": []}


class Harness(object):
    def __init__(self, answer=True, fail=()):
        self.answer = answer
        self.fail = set(fail)
        self.written = []
        self.events = []
        self.asked = 0

    def writer(self, verdict):
        tid = verdict["task"]["id"]
        self.events.append(("write", tid))
        if tid in self.fail:
            raise RuntimeError("boom")
        self.written.append(tid)
        return []

    def ask(self, lines):
        self.asked += 1
        self.events.append(("ask", None))
        return self.answer

    def send(self, reports):
        for r in reports:
            self.events.append(("report", r["task_id"], r["status"]))
        return True, ""

    def go(self, tasks, facts=None):
        return apply_run.run(tasks, facts or Facts(), self.writer, self.ask, self.send, lambda line: None)


class Run(unittest.TestCase):
    def test_clear_tasks_are_written_and_completed_without_a_question(self):
        h = Harness()
        sent, _ = h.go([task("a"), task("b", "2")])
        self.assertEqual(h.asked, 0)
        self.assertEqual(h.written, ["a", "b"])
        self.assertEqual([r["status"] for r in sent], ["completed", "completed"])

    def test_an_aborted_task_is_reported_before_any_work(self):
        h = Harness()
        h.go([task("a"), task("x", "gone")])
        first_write = h.events.index(("write", "a"))
        self.assertIn(("report", "x", "aborted"), h.events[:first_write])

    def test_declining_the_warnings_aborts_those_tasks_and_runs_the_rest(self):
        facts = Facts(families={("Bed", "S")})
        warned = task("w", "2", items=[{"family_name": "Chair", "type_name": "C"}])
        h = Harness(answer=False)
        sent, _ = h.go([task("a"), warned], facts)
        self.assertEqual(h.asked, 1)
        self.assertEqual(h.written, ["a"])
        statuses = dict((r["task_id"], r["status"]) for r in sent)
        self.assertEqual(statuses["w"], "aborted")

    def test_proceeding_runs_a_warned_task_and_reports_the_warning(self):
        facts = Facts(families={("Bed", "S")})
        warned = task("w", "2", items=[{"family_name": "Chair", "type_name": "C"}])
        h = Harness(answer=True)
        sent, _ = h.go([warned], facts)
        self.assertEqual(h.written, ["w"])
        self.assertEqual(sent[0]["status"], "completed_with_warnings")

    def test_a_writer_error_fails_that_task_and_the_next_still_runs(self):
        h = Harness(fail={"a"})
        sent, _ = h.go([task("a"), task("b", "2")])
        statuses = dict((r["task_id"], r["status"]) for r in sent)
        self.assertEqual(statuses, {"a": "failed", "b": "completed"})

    def test_each_task_is_reported_as_soon_as_it_is_done(self):
        h = Harness()
        h.go([task("a"), task("b", "2")])
        self.assertLess(h.events.index(("report", "a", "completed")), h.events.index(("write", "b")))

    def test_every_claimed_task_ends_with_a_report(self):
        h = Harness()
        tasks = [task("a"), task("x", "gone"), task("c", "3")]
        sent, _ = h.go(tasks)
        self.assertEqual(sorted(r["task_id"] for r in sent), ["a", "c", "x"])

    def test_a_check_that_raises_frees_every_claimed_task_with_the_reason(self):
        h = Harness()
        lines = []

        def planner(tasks):
            raise RuntimeError("settings unreadable")

        sent, told = apply_run.run(
            [task("a"), task("b", "2")], Facts(), h.writer, h.ask, h.send, lambda line: None,
            planner=planner, log=lines.append)
        self.assertEqual([(r["task_id"], r["status"]) for r in sent], [("a", "failed"), ("b", "failed")])
        self.assertIn("settings unreadable", sent[0]["messages"][0])
        self.assertEqual(h.written, [])
        self.assertTrue(any("check of the tasks failed" in line for line in lines))

    def test_the_log_gets_each_step_including_a_writers_traceback(self):
        h = Harness(fail={"a"})
        lines = []
        apply_run.run([task("a")], Facts(), h.writer, h.ask, h.send, lambda line: None, log=lines.append)
        text = "\n".join(lines)
        self.assertIn("task a (1): writing", text)
        self.assertIn("RuntimeError: boom", text)
        self.assertIn("1 task(s): 1 clear", text)

    def test_a_log_that_raises_never_fails_the_run(self):
        h = Harness()

        def broken(line):
            raise IOError("disk full")

        sent, _ = apply_run.run([task("a")], Facts(), h.writer, h.ask, h.send, lambda line: None, log=broken)
        self.assertEqual([r["status"] for r in sent], ["completed"])

    def test_no_tasks_does_nothing(self):
        h = Harness()
        self.assertEqual(h.go([]), ([], []))


if __name__ == "__main__":
    unittest.main()

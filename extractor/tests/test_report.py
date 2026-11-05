"""What a run says (`room_m.report`). Stdlib unittest, CPython."""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "pyRevit"))

from room_m import report  # noqa: E402


class FakeResult(object):
    def __init__(self, lines=(), status=True):
        self._lines, self.status = list(lines), status

    def message_as_list(self):
        return list(self._lines)

    def append_message(self, line):
        self._lines.append(line)

    def update_sep(self, status, line):
        self.status = self.status and status
        self._lines.append(line)


def collect():
    out = []
    return out, out.append


class Items(unittest.TestCase):
    def test_an_item_line_names_what_was_done_and_how_it_went(self):
        out, sink = collect()
        report.begin(None, sink)
        report.item("exported", "sheet", "101 Ground Plan")
        report.item("exported", "sheet", "102 Roof", ok=False, detail="no title block")
        self.assertEqual(["exported sheet 101 Ground Plan [ok]", "exported sheet 102 Roof: no title block [FAILED]"], out)

    def test_counts_are_singular_and_plural(self):
        self.assertEqual("1 room", report.noun(1, "room"))
        self.assertEqual("26 rooms", report.noun(26, "room"))


class IngestReply(unittest.TestCase):
    def reply(self, **extra):
        body = {"accepted": True, "room_count": 32, "snapshot_taken_at": "t",
                "models": [{"model_id": "House A", "room_count": 32}]}
        body.update(extra)
        return json.dumps(body)

    def test_a_live_model_is_one_readable_line_not_json(self):
        lines = report.ingest_lines("rooms", self.reply())
        self.assertEqual([(report.OK, "rooms: House A - 32 rooms stored")], lines)

    def test_a_held_model_says_so_and_why(self):
        reply = self.reply(accepted=False, models=[
            {"model_id": "House A", "room_count": 32, "quarantined": "phase disagrees"}])
        (state, line), = report.ingest_lines("rooms", reply)
        self.assertEqual(report.HELD, state)
        self.assertIn("NOT live", line)
        self.assertIn("phase disagrees", line)

    def test_each_entity_finds_its_own_count_key(self):
        reply = json.dumps({"accepted": True, "door_count": 1, "models": [{"model_id": "M", "door_count": 1}]})
        self.assertEqual("doors: M - 1 door stored", report.ingest_lines("doors", reply)[0][1])

    def test_a_reply_without_models_falls_back_to_the_total(self):
        reply = json.dumps({"accepted": True, "ffe_count": 5})
        self.assertEqual((report.OK, "FF&E: 5 FF&E stored"), report.ingest_lines("FF&E", reply)[0])

    def test_a_reply_that_is_not_json_is_shown_short(self):
        (state, line), = report.ingest_lines("rooms", "x" * 500)
        self.assertIsNone(state)
        self.assertLess(len(line), 220)

    def test_say_ingest_records_on_the_result_and_reports_whether_all_went_live(self):
        out, sink = collect()
        report.begin(None, sink)
        result = FakeResult()
        self.assertTrue(report.say_ingest("rooms", self.reply(), result))
        self.assertEqual(["rooms: House A - 32 rooms stored [ok]"], out)
        self.assertEqual(out, result.message_as_list())
        held = self.reply(models=[{"model_id": "M", "room_count": 1, "quarantined": "why"}])
        self.assertFalse(report.say_ingest("rooms", held, result))


class ServerFailure(unittest.TestCase):
    def setUp(self):
        import tempfile
        from room_m import task_log
        self.task_log = task_log
        self.dir = tempfile.mkdtemp()
        self.path = task_log.start("test run", directory=self.dir)

    def tearDown(self):
        import shutil
        self.task_log.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_the_person_gets_the_number_and_the_log_path_and_the_log_gets_the_whole_reply(self):
        out, sink = collect()
        result = FakeResult(status=True)
        report.begin(result, sink)
        reply = "FULL REPLY " + "x" * 400
        line = report.server_failure("Sheet push", 422, reply, result)
        self.assertIn("error 422", line)
        self.assertIn(self.path, line)
        self.assertNotIn("FULL REPLY", line, "the reply belongs in the log, not on the screen")
        self.assertTrue(out[0].endswith(report.FAILED))
        self.assertFalse(result.status, "the run goes red")
        with open(self.path) as handle:
            self.assertIn("FULL REPLY", handle.read())

    def test_no_answer_at_all_is_said_as_unreachable(self):
        out, sink = collect()
        report.begin(None, sink)
        line = report.server_failure("rooms push", None, "connection refused")
        self.assertIn("could not be reached", line)

    def test_a_failed_run_ends_by_naming_the_log(self):
        out, sink = collect()
        result = FakeResult(["build"], status=False)
        report.begin(result, sink)
        report.finish(result, "export")
        self.assertIn("Log file: {}".format(self.path), out)


class Finish(unittest.TestCase):
    def test_a_clean_run_ends_with_one_clear_line(self):
        out, sink = collect()
        result = FakeResult(["build"])
        report.begin(result, sink)
        self.assertTrue(report.finish(result, "export"))
        self.assertEqual("Finished: export - completed with no problems.", out[-1])

    def test_a_failed_run_says_so_and_nothing_recorded_stays_hidden(self):
        out, sink = collect()
        result = FakeResult(["build"])
        report.begin(result, sink)
        result.append_message("House A: failed with exception: boom")
        result.status = False
        self.assertFalse(report.finish(result, "export"))
        self.assertIn("House A: failed with exception: boom", out)
        self.assertIn("WITH PROBLEMS", out[-1])

    def test_a_line_already_said_is_not_said_again_and_the_build_lines_are_not_repeated(self):
        out, sink = collect()
        result = FakeResult(["build line"])
        report.begin(result, sink)
        result.append_message(report.item("exported", "sheet", "101"))
        report.finish(result, "sheets")
        self.assertEqual(1, sum(1 for line in out if line.startswith("exported sheet 101")))
        self.assertNotIn("build line", out)


class ServerLabel(unittest.TestCase):
    def test_the_header_small_print_names_the_server_or_says_none_is_set(self):
        from room_m import server_config as sc
        self.assertEqual("Server: not set", sc.short(environ={}, path=os.path.join(os.devnull, "x")))
        self.assertEqual("Server: http://192.168.0.97:5151", sc.short(environ={"ROOMMATE_SERVER": "192.168.0.97"}))


class LogMirror(unittest.TestCase):
    def test_every_line_said_and_the_closing_line_are_in_the_log(self):
        import shutil
        import tempfile

        from room_m import task_log

        folder = tempfile.mkdtemp()
        try:
            path = task_log.start("Blueprint export", os.path.join(folder, "RoomMate"))
            said = []
            result = FakeResult()
            report.begin(result, sink=said.append)
            report.say("pushed blueprint WC", report.OK)
            report.say("blueprint KITCHEN refused", report.FAILED)
            result.update_sep(False, "x")
            report.finish(result, "blueprint export")
            with open(path) as handle:
                text = handle.read()
            self.assertIn("pushed blueprint WC [ok]", text)
            self.assertIn("ERROR | ", text)
            self.assertIn("blueprint KITCHEN refused [FAILED]", text)
            self.assertIn("Finished: blueprint export - completed WITH PROBLEMS", text)
        finally:
            task_log.stop()
            shutil.rmtree(folder, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()

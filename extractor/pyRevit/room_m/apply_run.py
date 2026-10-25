"""Run a claimed list of room tasks: check, ask once, write, report. No Revit in it.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. The things
that touch Revit (`facts`, `writer`) and the things that touch the person or the server (`ask`,
`send`) are passed in, so `extractor/tests/test_apply_run.py` drives the whole sequence with
fakes. The order matters and is the point of this module:

1. **Check the whole list** (`task_plan.plan_tasks`) before anything is written.
2. **Ask once**, only when some task has warnings: proceed with them, or abort those tasks.
3. **Report what will not run BEFORE any work**: an aborted task, and a warned task the person
   declined. The server holds every claimed task `in_progress`, so a task nobody reports is stuck
   until a person resends it.
4. **Write each remaining task**, and report it as soon as it is done, so a session that dies
   halfway has reported everything before the one it died in.
5. **Nothing is left unreported**: a task the loop never reached is reported failed.
"""

from room_m import task_plan


def run(tasks, facts, writer, ask_proceed, send, say):
    """Returns `(reports, lines)`: every report sent, and what to show.

    `writer(verdict)` writes one task and returns its notes (a list of strings); it raises to
    fail the task, and must leave the model as it found it when it does. `ask_proceed(lines)`
    returns True to proceed with warned tasks. `send(reports)` posts reports and returns
    `(ok, text)`. `say(line)` shows a line as it happens."""
    sent = []
    lines = []

    def tell(line):
        lines.append(line)
        say(line)

    def post(reports):
        if not reports:
            return
        ok, text = send(reports)
        sent.extend(reports)
        if not ok:
            tell("The server did not accept the report: {}".format(text))

    if not tasks:
        return sent, lines

    verdicts = task_plan.plan_tasks(tasks, facts)
    clear, warned, aborted = task_plan.summarise(verdicts)
    tell("{} task(s): {} clear, {} with warnings, {} aborted".format(len(tasks), clear, warned, aborted))

    proceed = True
    if task_plan.has_warnings(verdicts):
        warning_lines = []
        for v in verdicts:
            if v["warnings"] and not v["aborts"]:
                warning_lines.extend(f["message"] for f in v["warnings"])
        proceed = bool(ask_proceed(warning_lines))

    for v in verdicts:
        for f in v["aborts"]:
            tell("aborted: {}".format(f["message"]))
    post(task_plan.aborted_reports(verdicts, proceed))
    if not proceed:
        tell("Stopped at the warnings: nothing was written for those tasks.")

    for v in task_plan.runnable(verdicts, proceed):
        label = (v["task"].get("room") or {}).get("number") or v["task"].get("id")
        try:
            notes = writer(v)
            report = task_plan.result_report(v, notes=notes)
        except Exception as e:
            report = task_plan.result_report(v, error=e)
        tell("{}: {}".format(label, report["status"]))
        for m in report["messages"]:
            tell("    {}".format(m))
        post([report])

    reported = set(r["task_id"] for r in sent)
    stranded = [
        {"task_id": t.get("id"), "status": "failed", "messages": ["the session ended before this task was reached"]}
        for t in tasks
        if t.get("id") not in reported
    ]
    post(stranded)
    return sent, lines

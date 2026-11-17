"""One run of the room tasks for one model: claim, hand each kind to its handler, leave nothing claimed. No Revit in it.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

**Why this is its own module.** The Tasks button and an unattended run (`room_mate.tasks_unattended`, for a batch
flow that opens one model after another) differ only in how the tasks are CHOSEN (a person ticks them, or every
queued one is taken) and in who answers the two questions a run may ask (proceed with warned tasks; the RLS
settings are incomplete). The part that keeps the server honest is the same for both and is written once here:
every claimed task is reported, by its handler, as unhandled, or as stranded when a handler raised or the run
stopped, because the server holds a claimed task `in_progress` and nothing else would free it.

The questions are answered by a `prompts` object the handlers are given (`ask_proceed(noun, lines)`,
`fix_rls_settings(doc, problems)`, `say(line)`); `Unattended` is the one that asks nobody. Pure, so it is tested
on CPython in `extractor/tests/test_task_session.py`.
"""

from room_m import task_dispatch

UNHANDLED = "this plug-in has no handler for this kind of task"
STRANDED = "the session ended before this task was reached"


def run(claim, handlers, free, log=None):
    """Claim, dispatch and report. Returns a dict:

    - `claimed`: the tasks the claim brought (empty when it failed or there were none);
    - `claim_error`: why the claim failed (`"<status>: <text>"`), else None, with `claim_status` (the HTTP
      code, None for no answer) and `claim_text` (the reply) for a caller that reports it;
    - `reports`: every report a handler or this function sent;
    - `unhandled`: how many claimed tasks had a kind no handler knows;
    - `errors`: one line per handler that raised.

    `claim()` returns `(ok, status, text, tasks)` (`post_tasks.claim_tasks`). `handlers` maps a handler name
    (`task_dispatch.APPLY`, `RLS`) to `fn(tasks) -> reports sent`. `free(tasks, status, message)` reports tasks
    and returns the reports; it must not raise. `log(level, line)` is told each step and never fails a run.

    Whatever happens after the claim, including an exception, which is raised again, every claimed task that no
    report names is freed as failed."""
    out = {"claimed": [], "claim_error": None, "claim_status": None, "claim_text": None,
           "reports": [], "unhandled": 0, "errors": []}
    reported = set()

    def note(level, line):
        if log is not None:
            try:
                log(level, line)
            except Exception:
                pass

    def record(reports):
        reports = reports or []
        out["reports"].extend(reports)
        reported.update(r.get("task_id") for r in reports)

    note("step", "claiming the tasks")
    ok, status, text, claimed = claim()
    if not ok:
        out["claim_error"] = "{}: {}".format(status if status is not None else "no response", text)
        out["claim_status"], out["claim_text"] = status, text
        note("error", "claim failed: {}".format(out["claim_error"]))
        return out
    claimed = claimed or []
    out["claimed"] = claimed
    if not claimed:
        note("info", "nothing was queued")
        return out
    note("info", "claimed {}".format(task_dispatch.count_by_kind(claimed)))
    try:
        groups, unhandled = task_dispatch.split(claimed)
        out["unhandled"] = len(unhandled)
        if unhandled:
            record(free(unhandled, "failed", UNHANDLED))
        for name, group in groups:
            note("step", "{}: {} task(s)".format(name, len(group)))
            handler = handlers.get(name)
            if handler is None:
                # a handler in task_dispatch with no function here: the group is stranded, and freed below
                out["errors"].append("{}: no function runs this handler".format(name))
                continue
            try:
                record(handler(group))
            except Exception as e:
                note("exception", "{} failed".format(name))
                out["errors"].append("{} failed with exception: {}".format(name, e))
    finally:
        stranded = [t for t in claimed if t.get("id") not in reported]
        if stranded:
            note("warning", "{} claimed task(s) were never reported; reporting them failed".format(len(stranded)))
            record(free(stranded, "failed", STRANDED))
    return out


class Unattended(object):
    """The answers of a run nobody watches: a batch flow's.

    - Warned tasks are written only when `proceed_with_warnings` is set, else reported aborted (the planner's
      own rule for a warning a person declined). Off by default: a warning is the planner saying "look first".
    - Incomplete or wrong RLS settings are never edited: the create-RLS tasks are reported aborted with what is
      missing, and the model's settings are left as they were. A person fixes them with the RLS settings button.
    - Everything is said through `say` (the flow's console)."""

    def __init__(self, proceed_with_warnings=False, say=None):
        self.proceed_with_warnings = bool(proceed_with_warnings)
        self._say = say

    def say(self, line):
        if self._say is not None:
            try:
                self._say(line)
            except Exception:
                pass

    def ask_proceed(self, noun, lines):
        verdict = "proceeding with them" if self.proceed_with_warnings else "aborting those (proceed_with_warnings is off)"
        self.say("Some {} have warnings; {}:".format(noun, verdict))
        for line in lines:
            self.say("  {}".format(line))
        return self.proceed_with_warnings

    def fix_rls_settings(self, doc, problems):
        self.say("The RLS settings of this model need attention, and an unattended run does not edit them:")
        for line in problems:
            self.say("  {}".format(line))
        return None

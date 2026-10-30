"""Check a claimed list of room tasks before anything is written (plan decision 22).

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so
that CPython runs it too, which is the point: it imports nothing from Revit, so
`extractor/tests/test_task_plan.py` exercises every rule on a CI runner. What Revit is asked is
behind a `facts` object (below); the planner decides what the answers MEAN.

**The whole list is checked first; any abort is reported before work starts.** A task is one
of three things after checking: clear, warned (its families will be short of what the
task asked for) or aborted (nothing of it will be written). Warnings are put to the person once,
for the whole list, with two choices: abort those tasks, or ignore the warnings and proceed.

`facts` answers, for the model open in Revit:
  phase_exists(phase_name) -> bool
  find_room(number, room_id) -> a room handle or None. The room is found by its `Number`
      property, and the id is the fallback when the task carries no number.
  room_has_content(room, phase_name) -> bool  (FF&E placed in the room in that phase)
  family_available(family_name, type_name) -> bool  (in the model or the library location)
"""

POPULATE = "populate_room"
CREATE_RLS = "create_rls"

# What a finding is: `code` is what a test or a report reader matches, `message` is for people.
PHASE_MISSING = "phase_missing"
ROOM_MISSING = "room_missing"
ROOM_HAS_CONTENT = "room_has_content"
SHEETS_ELSEWHERE = "sheets_elsewhere"
FAMILY_MISSING = "family_missing"
NO_SHEETS = "no_sheets"
UNKNOWN_KIND = "unknown_kind"


def finding(code, message):
    return {"code": code, "message": message}


def _room_label(task):
    room = task.get("room") or {}
    return room.get("number") or room.get("name") or room.get("room_id") or "?"


def _check_one(task, facts):
    """`(aborts, warnings)` for one task."""
    aborts = []
    warnings = []
    kind = task.get("kind")
    label = _room_label(task)

    if kind not in (POPULATE, CREATE_RLS):
        return [finding(UNKNOWN_KIND, "room {}: this plug-in does not know a task of kind '{}'".format(label, kind))], []

    phase = task.get("phase")
    if not facts.phase_exists(phase):
        aborts.append(finding(PHASE_MISSING, "phase '{}' does not exist in this model".format(phase)))

    room_ref = task.get("room") or {}
    room = facts.find_room(room_ref.get("number"), room_ref.get("room_id"))
    if room is None:
        aborts.append(finding(ROOM_MISSING, "room {} no longer exists in this model".format(label)))

    # What needs the room and phase to be real is only asked once they are.
    if kind == POPULATE and not aborts:
        if not task.get("overwrite_layout") and facts.room_has_content(room, phase):
            aborts.append(
                finding(
                    ROOM_HAS_CONTENT,
                    "room {} already has content in phase '{}' and this task does not overwrite it".format(label, phase),
                )
            )

    if kind == CREATE_RLS:
        if not task.get("sheets"):
            aborts.append(finding(NO_SHEETS, "room {}: the task carries no sheet layout".format(label)))
    elif task.get("create_rls"):
        # Sheets are made by the RLS create button from a create_rls task (room_m.rls_task);
        # this button places items only.
        warnings.append(
            finding(SHEETS_ELSEWHERE, "room {}: this task asks for sheets too; this button places items only, so make the sheets with a create_rls task".format(label))
        )

    if kind == POPULATE:
        seen = set()
        for item in task.get("items") or []:
            key = (item.get("family_name"), item.get("type_name"))
            if key in seen:
                continue
            seen.add(key)
            if not facts.family_available(key[0], key[1]):
                warnings.append(
                    finding(
                        FAMILY_MISSING,
                        "room {}: family '{}' type '{}' is in neither the model nor the library".format(label, key[0] or "?", key[1]),
                    )
                )
    return aborts, warnings


def plan_tasks(tasks, facts):
    """Check every task. Returns a list of verdicts in the order given, each
    `{"task": task, "aborts": [...], "warnings": [...]}`."""
    verdicts = []
    for task in tasks:
        aborts, warnings = _check_one(task, facts)
        verdicts.append({"task": task, "aborts": aborts, "warnings": warnings})
    return verdicts


def has_warnings(verdicts):
    return any(v["warnings"] and not v["aborts"] for v in verdicts)


def _messages(findings):
    return [f["message"] for f in findings]


def aborted_reports(verdicts, proceed):
    """The reports to send BEFORE any work: a task that aborted on its own checks, and, when the
    person chose not to proceed, every task that only had warnings, with the reason."""
    reports = []
    for v in verdicts:
        task_id = v["task"].get("id")
        if v["aborts"]:
            reports.append({"task_id": task_id, "status": "aborted", "messages": _messages(v["aborts"])})
        elif v["warnings"] and not proceed:
            msgs = ["stopped at the warnings"] + _messages(v["warnings"])
            reports.append({"task_id": task_id, "status": "aborted", "messages": msgs})
    return reports


def runnable(verdicts, proceed):
    """The verdicts to write, in order: those with no abort, and with no warning unless the
    person chose to proceed."""
    out = []
    for v in verdicts:
        if v["aborts"]:
            continue
        if v["warnings"] and not proceed:
            continue
        out.append(v)
    return out


def result_report(verdict, error=None, notes=None):
    """The report for a task that was run. `error` is what went wrong (the task is failed);
    otherwise a task that had warnings, or that wrote with `notes`, is completed with warnings."""
    task_id = verdict["task"].get("id")
    if error:
        return {"task_id": task_id, "status": "failed", "messages": [str(error)]}
    messages = _messages(verdict["warnings"]) + list(notes or [])
    status = "completed_with_warnings" if messages else "completed"
    return {"task_id": task_id, "status": status, "messages": messages}


def summarise(verdicts):
    """Counts for the dialog: `(clear, warned, aborted)`."""
    aborted = sum(1 for v in verdicts if v["aborts"])
    warned = sum(1 for v in verdicts if v["warnings"] and not v["aborts"])
    return len(verdicts) - aborted - warned, warned, aborted

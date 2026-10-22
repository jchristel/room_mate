"""Check a claimed list of room tasks before anything is written (plan decision 22).

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so
that CPython runs it too, which is the point: it imports nothing from Revit, so
`extractor/tests/test_task_plan.py` exercises every rule on a CI runner. What Revit is asked is
behind a `facts` object (below); the planner decides what the answers MEAN.

**The whole list is checked first; any abort is reported before work starts.** A task is one
of three things after checking: clear, warned (its sheets or families will be short of what the
task asked for) or aborted (nothing of it will be written). Warnings are put to the person once,
for the whole list, with two choices: abort those tasks, or ignore the warnings and proceed.

`facts` answers, for the model open in Revit:
  phase_exists(phase_name) -> bool
  find_room(number, room_id) -> a room handle or None. The room is found by its `Number`
      property, and the id is the fallback when the task carries no number.
  room_has_content(room, phase_name) -> bool  (FF&E placed in the room in that phase)
  rls_number(task) -> the number the task's RLS will take, generated from the target room
  rls_exists(number) -> bool
  titleblock_available(sheet) -> bool  (a sheet family for the size the layout asks for)
  family_available(family_name, type_name) -> bool  (in the model or the library location)
"""

POPULATE = "populate_room"
CREATE_RLS = "create_rls"

# What a finding is: `code` is what a test or a report reader matches, `message` is for people.
PHASE_MISSING = "phase_missing"
ROOM_MISSING = "room_missing"
ROOM_HAS_CONTENT = "room_has_content"
NO_TITLEBLOCK = "no_titleblock"
RLS_EXISTS = "rls_exists"
RLS_DUPLICATE = "rls_duplicate_in_list"
FAMILY_MISSING = "family_missing"
NO_SHEETS = "no_sheets"
UNKNOWN_KIND = "unknown_kind"


def finding(code, message):
    return {"code": code, "message": message}


def _room_label(task):
    room = task.get("room") or {}
    return room.get("number") or room.get("name") or room.get("room_id") or "?"


def _wants_rls(task):
    """Whether the task is to make sheets at all."""
    if task.get("kind") == CREATE_RLS:
        return True
    return bool(task.get("create_rls"))


def _check_one(task, facts, rls_taken):
    """`(aborts, warnings, skip_sheets)` for one task. `rls_taken` holds the numbers earlier
    tasks of this list will use, so two tasks never both make the same sheet."""
    aborts = []
    warnings = []
    skip_sheets = False
    kind = task.get("kind")
    label = _room_label(task)

    if kind not in (POPULATE, CREATE_RLS):
        return [finding(UNKNOWN_KIND, "room {}: this plug-in does not know a task of kind '{}'".format(label, kind))], [], False

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

    if _wants_rls(task):
        sheets = task.get("sheets") or []
        if not sheets:
            # Nothing to make: not a reason to stop the layout, but the person should know.
            if kind == CREATE_RLS:
                aborts.append(finding(NO_SHEETS, "room {}: the task carries no sheet layout".format(label)))
            else:
                warnings.append(finding(NO_SHEETS, "room {}: the blueprint has no sheets, so no RLS will be made".format(label)))
                skip_sheets = True
        else:
            for sheet in sheets:
                if not facts.titleblock_available(sheet):
                    aborts.append(
                        finding(NO_TITLEBLOCK, "room {}: no sheet family for the size of sheet '{}'".format(label, _sheet_name(sheet)))
                    )
            if not aborts and not skip_sheets:
                number = facts.rls_number(task)
                if number in rls_taken:
                    warnings.append(
                        finding(RLS_DUPLICATE, "room {}: another task in this list makes sheet {}, so this one's sheets are skipped".format(label, number))
                    )
                    skip_sheets = True
                elif facts.rls_exists(number) and not task.get("overwrite_rls"):
                    warnings.append(
                        finding(RLS_EXISTS, "room {}: sheet {} already exists, so the sheets are skipped".format(label, number))
                    )
                    skip_sheets = True
                else:
                    rls_taken.add(number)

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
    return aborts, warnings, skip_sheets


def _sheet_name(sheet):
    return sheet.get("sheet_name") or sheet.get("sheet_number") or sheet.get("revit_sheet_id") or "?"


def plan_tasks(tasks, facts):
    """Check every task. Returns a list of verdicts in the order given, each
    `{"task": task, "aborts": [...], "warnings": [...], "skip_sheets": bool}`."""
    rls_taken = set()
    verdicts = []
    for task in tasks:
        aborts, warnings, skip_sheets = _check_one(task, facts, rls_taken)
        verdicts.append({"task": task, "aborts": aborts, "warnings": warnings, "skip_sheets": skip_sheets})
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

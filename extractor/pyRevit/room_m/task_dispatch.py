"""Which code handles which kind of room task. No Revit in it.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

The Tasks button asks the server for every kind of task in one claim and hands each kind to the
code that knows it: `APPLY` places a room's items (`room_m.apply_write`), `RLS` makes a room layout
sheet (`room_m.rls_write`). **A new kind of task is one line in `HANDLER_OF`** and a function in
`room_mate` for the handler, not another button. A claimed task nobody handles is reported failed
by name (`unhandled_reports`), because the server holds every claimed task `in_progress` and
nothing else would free it.

Items are placed before sheets are made, so that a `create_rls` task in the same claim finds
the room as the layout left it: `ORDER`.
"""

APPLY = "apply"
RLS = "rls"

HANDLER_OF = {
    "populate_room": APPLY,
    "update_room": APPLY,
    "create_rls": RLS,
}

ORDER = (APPLY, RLS)


def claim_kinds():
    """The kinds to ask the server for: every one that has a handler."""
    return sorted(HANDLER_OF)


def split(tasks):
    """`(groups, unhandled)`: `groups` is `[(handler, [tasks])]` in `ORDER`, leaving out a handler with
    none; `unhandled` are the tasks whose kind has no handler."""
    by_handler = dict((h, []) for h in ORDER)
    unhandled = []
    for task in tasks:
        handler = HANDLER_OF.get(task.get("kind"))
        if handler is None:
            unhandled.append(task)
        else:
            by_handler[handler].append(task)
    return [(h, by_handler[h]) for h in ORDER if by_handler[h]], unhandled


def count_by_kind(tasks):
    """`"2 create_rls, 1 populate_room"`: what a claim brought, for the log and the output."""
    counts = {}
    for task in tasks:
        kind = task.get("kind") or "?"
        counts[kind] = counts.get(kind, 0) + 1
    return ", ".join("{} {}".format(counts[k], k) for k in sorted(counts))


def failed_report(task, message):
    """A report that frees a claimed task nobody could run."""
    return {"task_id": task.get("id"), "status": "failed", "messages": [message]}


def unhandled_reports(tasks):
    return [
        failed_report(t, "this plug-in has no handler for a task of kind '{}'".format(t.get("kind")))
        for t in tasks
    ]

"""The task chooser's rules: how a task reads in the list and which tasks a filter shows. No Revit, no WPF.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so that CPython
runs it too (`extractor/tests/test_tasks_form.py`); the window (`tasks_window`) only draws it.

The list is what the server holds QUEUED for the open model (`post_tasks.list_queued`), read without claiming
anything: only the tasks a person ticks are claimed and run. A task's level is not on the task, so the button
works it out from the open model's rooms (`room_mate._room_level_names`) and hands this module a function.
"""

KIND_ALL = "all"
KIND_RLS = "create_rls"
KIND_OTHER = "other"

# (key, label) in the order the drop-down shows them
KIND_FILTERS = [
    (KIND_ALL, "All kinds"),
    (KIND_RLS, "Create RLS"),
    (KIND_OTHER, "Other (populate, update)"),
]

ALL_LEVELS = "All levels"
NO_LEVEL = "(level not found)"

_KIND_LABELS = {
    "create_rls": "Create RLS",
    "populate_room": "Populate room",
    "update_room": "Update room",
}


def kind_label(kind):
    return _KIND_LABELS.get(kind) or str(kind)


def row_of(summary, level_of):
    """One list row from a server task summary. `level_of(room)` answers the room's level name, or
    nothing when the open model has no such room."""
    room = summary.get("room") or {}
    number = room.get("number") or ""
    name = room.get("name") or ""
    kind = summary.get("kind") or ""
    level = level_of(room) or NO_LEVEL
    blueprint = (summary.get("blueprint") or {}).get("name") or ""
    label = " ".join(p for p in (number, name) if p) or "(room {})".format(room.get("room_id"))
    text = "{}  -  {}".format(label, kind_label(kind))
    if blueprint:
        text += "  -  blueprint {}".format(blueprint)
    return {
        "id": summary.get("id"),
        "kind": kind,
        "level": level,
        "text": text + "  -  " + level,
        "search": " ".join((label, kind_label(kind), blueprint, summary.get("id") or "")).lower(),
    }


def rows(summaries, level_of):
    """The rows, by level then room number, so a level's rooms sit together."""
    out = [row_of(s, level_of) for s in summaries]
    return sorted(out, key=lambda r: (r["level"] == NO_LEVEL, r["level"], r["text"]))


def level_choices(all_rows):
    """The level drop-down: everything, then each level present."""
    return [ALL_LEVELS] + sorted(set(r["level"] for r in all_rows), key=lambda n: (n == NO_LEVEL, n))


def shown(row, text, kind_key, level):
    """Whether `row` passes the filters: the text (any part of the room number or name, the kind, the
    blueprint or the task id, ignoring case), the kind, and the level."""
    if kind_key == KIND_RLS and row["kind"] != KIND_RLS:
        return False
    if kind_key == KIND_OTHER and row["kind"] == KIND_RLS:
        return False
    if level and level != ALL_LEVELS and row["level"] != level:
        return False
    needle = (text or "").strip().lower()
    return not needle or needle in row["search"]


def problems(ticked_count):
    return [] if ticked_count else ["Tick at least one task to run."]


def summary(ticked_count, shown_count, total):
    return "{} ticked; {} of {} shown".format(ticked_count, shown_count, total)

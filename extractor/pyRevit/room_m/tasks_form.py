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
    "create_rls_set": "Create RLS set",
    "populate_room": "Populate room",
    "update_room": "Update room",
}


# the kinds the "Create RLS" filter shows: one room's sheet, or a set of rooms' sheets
_RLS_KINDS = (KIND_RLS, "create_rls_set")


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
    # a set names its lead room and how many more; every room of it is found by the search
    others = [r for r in summary.get("set_rooms") or [] if r.get("room_id") != room.get("room_id")]
    every = " ".join(" ".join(p for p in (r.get("number") or "", r.get("name") or "") if p) for r in others)
    if others:
        label += " + {} room{}".format(len(others), "" if len(others) == 1 else "s")
    text = "{}  -  {}".format(label, kind_label(kind))
    if blueprint:
        text += "  -  blueprint {}".format(blueprint)
    return {
        "id": summary.get("id"),
        "kind": kind,
        "level": level,
        "text": text + "  -  " + level,
        "search": " ".join((label, every, kind_label(kind), blueprint, summary.get("id") or "")).lower(),
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
    if kind_key == KIND_RLS and row["kind"] not in _RLS_KINDS:
        return False
    if kind_key == KIND_OTHER and row["kind"] in _RLS_KINDS:
        return False
    if level and level != ALL_LEVELS and row["level"] != level:
        return False
    needle = (text or "").strip().lower()
    return not needle or needle in row["search"]


def problems(ticked_count):
    return [] if ticked_count else ["Check at least one task to run."]


def summary(ticked_count, shown_count, total):
    return "{} checked; {} of {} shown".format(ticked_count, shown_count, total)


def refresh_note(before_ids, after_ids):
    """What a refresh found, as a status line: how many tasks are queued now and how many are new or gone."""
    before, after = set(before_ids), set(after_ids)
    new, gone = len(after - before), len(before - after)
    line = "Refreshed: {} queued".format(len(after))
    if new:
        line += ", {} new".format(new)
    if gone:
        line += ", {} no longer queued".format(gone)
    return line + "."


def to_save(project, text, kind_key, level):
    """The section `prefs` keeps after a run: the project id and the filters (text, kind, level). Never the checked
    tasks: the queue is different next time."""
    return {"project": (project or {}).get("id"), "filter": text or "", "kind": kind_key or KIND_ALL,
            "level": level or ALL_LEVELS}


def remembered(saved, projects):
    """`(project index or None, filter text, kind index, level)` from the saved section. A project that no longer
    exists is dropped and an unknown kind is "all"; the level is checked against the levels once the tasks are read."""
    saved = saved or {}
    index = None
    for i, project in enumerate(projects):
        if project.get("id") == saved.get("project"):
            index = i
    kinds = [key for key, _label in KIND_FILTERS]
    kind = kinds.index(saved.get("kind")) if saved.get("kind") in kinds else 0
    return index, str(saved.get("filter") or ""), kind, str(saved.get("level") or ALL_LEVELS)

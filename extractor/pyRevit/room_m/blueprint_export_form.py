"""The blueprint export window's rules: what is valid, and how a choice reads. No Revit, no WPF.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so that
CPython runs it too (`extractor/tests/test_blueprint_export_form.py`); the window
(`blueprint_export_window`) only draws it.

**A blueprint is a room and its sheets together**, so a ticked room needs at least one sheet and a
name; a sheet belongs to one room, so one ticked for a room is not offered to another.
"""

from room_m import match_rule

# How many problems the status names before it says "and N more".
SHOWN = 4

# Shared with the sheet window: see `room_m.match_rule`.
sheets_caption = match_rule.numbers_caption
matches = match_rule.filter_matches
RELATIONS = match_rule.RELATIONS
make_rule = match_rule.make_rule
describe_rule = match_rule.describe_rule


def default_name(room):
    """The blueprint's name until the person types another: the room's name, else its label."""
    return (room.get("name") or room.get("label") or "").strip()


def taken_elsewhere(rows, mine):
    """The ids of the sheets ticked for any room other than `mine`. `rows` are
    `(room_id, [sheet ids])` pairs; a sheet is offered once."""
    taken = set()
    for room_id, sheet_ids in rows:
        if room_id != mine:
            taken.update(sheet_ids)
    return taken


def duplicate_names(rows):
    """Which of the ticked rooms share a blueprint name with another ticked room, as a set of positions in
    `rows` (`(name, ticked)` pairs). Only ticked rows count: every room starts with its own name, so the many
    unticked rooms called alike are not what is about to be pushed."""
    ticked = [i for i, (_name, on) in enumerate(rows) if on]
    names = [rows[i][0] for i in ticked]
    return set(ticked[j] for j in match_rule.duplicates(names))


def to_save(project, text, rule):
    """The section `prefs` keeps after an export: the project id, the room filter and the sheet rule."""
    return {"project": (project or {}).get("id"), "filter": text or "", "rule": rule}


def remembered(saved, projects):
    """`(project index or None, filter text, rule or None)` from the saved section; a project that no longer exists
    and a rule that no longer parses are dropped."""
    saved = saved or {}
    index = None
    for i, project in enumerate(projects):
        if project.get("id") == saved.get("project"):
            index = i
    return index, str(saved.get("filter") or ""), match_rule.rule_from_saved(saved.get("rule"))


def problems(project, phase, rooms):
    """What stops the export, as lines for the status; empty when it can run.

    `rooms` are the ticked rooms as `(label, name, sheet count)`."""
    out = []
    if project is None:
        out.append("Choose the project to export to.")
    if phase is None:
        out.append("Choose the phase the rooms are read in.")
    if not rooms:
        out.append("Check at least one room.")
    seen = {}
    for label, name, sheets in rooms:
        name = (name or "").strip()
        if not name:
            out.append("{}: give the blueprint a name.".format(label))
        elif name.lower() in seen:
            out.append("{}: the name '{}' is also {}'s.".format(label, name, seen[name.lower()]))
        else:
            seen[name.lower()] = label
        if sheets <= 0:
            out.append("{}: pick at least one room layout sheet.".format(label))
    if len(out) > SHOWN:
        out = out[:SHOWN] + ["and {} more".format(len(out) - SHOWN)]
    return out

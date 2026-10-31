"""The blueprint export window's rules: what is valid, and how a choice reads. No Revit, no WPF.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so that
CPython runs it too (`extractor/tests/test_blueprint_export_form.py`); the window
(`blueprint_export_window`) only draws it.

**A blueprint is a room and its sheets together**, so a ticked room needs at least one sheet and a
name; a sheet belongs to one room, so one ticked for a room is not offered to another.
"""

# How many problems the status names before it says "and N more".
SHOWN = 4


def sheets_caption(count):
    """What a room's sheet button says for `count` sheets ticked."""
    if count <= 0:
        return "(none)"
    return "1 sheet" if count == 1 else "{} sheets".format(count)


def matches(label, text):
    """Whether a room whose label is `label` is listed under the filter `text` (blank lists all)."""
    needle = (text or "").strip().lower()
    return not needle or needle in (label or "").lower()


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


def problems(project, phase, rooms):
    """What stops the export, as lines for the status; empty when it can run.

    `rooms` are the ticked rooms as `(label, name, sheet count)`."""
    out = []
    if project is None:
        out.append("Choose the project to export to.")
    if phase is None:
        out.append("Choose the phase the rooms are read in.")
    if not rooms:
        out.append("Tick at least one room.")
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

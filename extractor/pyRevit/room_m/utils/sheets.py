"""What the sheet buttons let a person choose from: the document's sheets, and the rooms of
one phase.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one, not
even inside a docstring.

**Nothing here guesses which sheets are room layout sheets, or which room a sheet is
for.** The export has no marker for either, so the person chooses (decision 19 in
docs/PLAN-blueprints.md). These functions only make the choosing possible: a labelled list
to pick from.
"""

from Autodesk.Revit.DB import BuiltInParameter

from duHast.Revit.Rooms.rooms import get_all_placed_rooms
from duHast.Revit.Views.sheets import get_all_sheets

from room_m.utils.generic import element_id_str
from room_m.utils.rooms import rooms_in_phase


def sheets_of(doc):
    """`[{"element", "id", "number", "name", "label"}]` for every real sheet of `doc`,
    ordered by sheet number.

    Placeholder sheets are left out: they have no views and nothing to export. A sheet
    whose number or name cannot be read is still offered, labelled by its id, rather than
    dropped without a word."""
    found = []
    for sheet in get_all_sheets(doc):
        try:
            if getattr(sheet, "IsPlaceholder", False):
                continue
        except Exception:
            pass
        try:
            number = str(sheet.SheetNumber)
        except Exception:
            number = ""
        try:
            name = str(sheet.Name)
        except Exception:
            name = ""
        sheet_id = element_id_str(sheet.Id)
        found.append(
            {
                "element": sheet,
                "id": sheet_id,
                "number": number,
                "name": name,
                "label": "{} - {}".format(number or "(no number)", name or "(no name)"),
            }
        )
    found.sort(key=lambda s: (s["number"].lower(), s["id"]))
    return _unique_labels(found)


def all_placed_rooms(doc):
    """The placed rooms of every phase, in the shape of `placed_rooms`, for the sheet export window:
    a sheet's rooms are said by the person and do not depend on a phase."""
    return _placed(doc, None)


def placed_rooms(doc, phase_name):
    """`[{"id", "number", "name", "level", "label"}]` for the PLACED rooms of one phase.

    A room with no area is not placed or not enclosed, and no blueprint can be made from it,
    so it is not offered. The phase is the rooms equality test (`rooms_in_phase`), not the
    doors range test."""
    return _placed(doc, rooms_in_phase(doc, phase_name))


def _placed(doc, allowed):
    """The placed rooms in the pick-list shape; only those whose id is in `allowed` when it is given."""
    found = []
    for room in get_all_placed_rooms(doc):
        room_id = element_id_str(room.Id)
        if allowed is not None and room_id not in allowed:
            continue
        try:
            name = room.get_Parameter(BuiltInParameter.ROOM_NAME).AsString() or ""
        except Exception:
            name = ""
        try:
            number = str(room.Number)
        except Exception:
            number = ""
        try:
            level = room.Level.Name if room.Level is not None else ""
        except Exception:
            level = ""
        found.append(
            {
                "element": room,
                "id": room_id,
                "number": number,
                "name": name,
                "level": level,
                "label": "{} {} ({})".format(number, name, level).strip(),
            }
        )
    found.sort(key=lambda r: (r["level"], r["number"], r["id"]))
    return _unique_labels(found)


def _unique_labels(items):
    """The pick list returns the label string, so two items sharing one would silently
    resolve to whichever was found first. A shared label carries the element id."""
    counts = {}
    for item in items:
        counts[item["label"]] = counts.get(item["label"], 0) + 1
    for item in items:
        if counts[item["label"]] > 1:
            item["label"] = "{}  [id {}]".format(item["label"], item["id"])
    return items


def pick(forms, items, title, button_name, multiselect=True):
    """Offer `items` (each with a `label`) and return the chosen ones, in list order, or
    `None` when the person cancelled. An empty choice is `[]`, a different answer."""
    by_label = dict((item["label"], item) for item in items)
    chosen = forms.SelectFromList.show(
        [item["label"] for item in items],
        title=title,
        button_name=button_name,
        multiselect=multiselect,
    )
    if chosen is None:
        return None
    if not multiselect:
        chosen = [chosen]
    wanted = set(chosen)
    return [item for item in items if item["label"] in wanted]

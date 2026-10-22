"""Read chosen Revit sheets as duHast exports them, ready to push to the sheet library.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

**The sheet is duHast's export, unchanged.** `convert_revit_sheet` is duHast's, and what it
returns (properties, viewports with their positions on the sheet, crop boxes, schedules) is
sent as it stands: this module does not re-measure a viewport. What it ADDS is the
identity the server needs and duHast does not carry as one thing -- the Revit sheet id and
the document title, together -- and the room the person chose.

**Tags are taken out of the sheet** (`room_m.sheet_tags`): the sheet that goes to the library
or a blueprint has none, and they come back as `pushed["tags"]` (observations) with what could
not be read in `pushed["tag_problems"]`. Both are the extractor's own keys: the pushers drop
them. Not in duHast's export, so not here: view template, view names. See
docs/PLAN-blueprints.md, track B.
"""

from room_m.post_common import duhast_to_plain
from room_m.sheet_tags import take_tags


def _tag_problems():
    """The tags duHast skipped while converting the sheet (see `to_data_tag.take_problems`);
    none when this duHast does not read tags."""
    try:
        from duHast.Revit.Annotation.Export.to_data_tag import take_problems
    except ImportError:
        return []
    return take_problems()


def read_sheet(doc, entry, room=None, phase_name=None):
    """One chosen sheet (an item of `room_m.utils.sheets.sheets_of`) as the pushed shape.

    Raises if duHast cannot convert the sheet; the caller reports that sheet and carries on
    with the others, so one unreadable sheet does not cost the run."""
    from duHast.Revit.Views.Export.sheets_to_data import convert_revit_sheet

    data = convert_revit_sheet(doc=doc, sheet=entry["element"])
    pushed = {
        "revit_sheet_id": entry["id"],
        "document_title": doc.Title,
        "sheet_number": entry["number"],
        "sheet_name": entry["name"],
        "sheet": duhast_to_plain(data),
    }
    pushed["tags"] = take_tags(
        pushed["sheet"],
        {"document_title": doc.Title, "revit_sheet_id": entry["id"], "sheet_number": entry["number"]},
    )
    pushed["tag_problems"] = _tag_problems()
    if room is not None:
        # The server holds the room as `{model_id, room_id}`; a model is its document's title,
        # as everywhere else in this extractor.
        pushed["room"] = {"model_id": doc.Title, "room_id": room["id"]}
    if phase_name:
        pushed["phase"] = phase_name
    return pushed


def read_sheets(doc, assignments, phase_name=None):
    """`assignments` is a list of `(sheet entry, room or None)`.

    Returns `(pushed, failures)`: the sheets that converted, and `(label, reason)` for each
    that did not."""
    pushed = []
    failures = []
    for entry, room in assignments:
        try:
            pushed.append(read_sheet(doc, entry, room, phase_name))
        except Exception as e:
            failures.append((entry["label"], str(e)))
    return pushed, failures

"""Read chosen Revit sheets as duHast exports them, ready to push to the sheet library.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

**The sheet is duHast's export, unchanged.** `convert_revit_sheet` is duHast's, and what it
returns (properties, viewports with their positions on the sheet, crop boxes, schedules) is
sent as it stands: this module does not re-measure a viewport. What it ADDS is the
identity the server needs and duHast does not carry as one thing -- the Revit sheet id and
the document title, together -- the paper size measured from the title block, and the figures of the
rooms the person ticked (`room_m.room_figures`). No room is linked to the sheet.

**Tags are taken out of the sheet** (`room_m.sheet_tags`): the sheet that goes to the library
or a blueprint has none, and they come back as `pushed["tags"]` (observations, each tagged item
carrying the type code read from its type when the project names a parameter) with what could
not be read in `pushed["tag_problems"]`. Both are the extractor's own keys: the pushers drop
them. Not in duHast's export, so not here: view template, view names. See
docs/PLAN-blueprints.md, track B.
"""

from room_m import room_figures
from room_m.post_common import duhast_to_plain
from room_m.sheet_tags import attach_type_codes, take_tags
from room_m.utils.tag_codes import type_code_reader


def _tag_problems():
    """The tags duHast skipped while converting the sheet (see `to_data_tag.take_problems`);
    none when this duHast does not read tags."""
    try:
        from duHast.Revit.Annotation.Export.to_data_tag import take_problems
    except ImportError:
        return []
    return take_problems()


def _size_mm(doc, entry):
    """The sheet's paper size from its title block, `{"width_mm", "height_mm"}`; None when the
    sheet has no title block or the box cannot be read."""
    try:
        from duHast.Revit.Views.sheets import get_title_block_size_mm
        size = get_title_block_size_mm(doc, entry["element"])
    except Exception:
        return None
    if size is None:
        return None
    return {"width_mm": size[0], "height_mm": size[1]}


def read_sheet(doc, entry, room=None, phase_name=None, rooms=None, type_code_parameter=None):
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
    if type_code_parameter and pushed["tags"]:
        pushed["tags_without_code"] = attach_type_codes(pushed["tags"], type_code_reader(doc, type_code_parameter))
    if room is not None:
        # The server holds the room as `{model_id, room_id}`; a model is its document's title,
        # as everywhere else in this extractor.
        pushed["room"] = {"model_id": doc.Title, "room_id": room["id"]}
    if phase_name:
        pushed["phase"] = phase_name
    size = _size_mm(doc, entry)
    if size is not None:
        pushed["size_mm"] = size
    if rooms:
        # The rooms this sheet covers are said by the person; their figures travel with the
        # sheet, and no room is linked to it.
        read = [room_figures.read_room(doc, r) for r in rooms]
        figure = room_figures.combine(read)
        if figure is not None:
            pushed["room_summary"] = figure
        pushed["rooms"] = [room_figures.room_label(r) for r in rooms]
    return pushed


def read_sheets(doc, assignments, phase_name=None, type_code_parameter=None):
    """`assignments` is a list of `(sheet entry, room or None)` or `(sheet entry, room, rooms)`,
    where `rooms` are the placed rooms (`room_m.utils.sheets.placed_rooms`) the sheet covers.

    Returns `(pushed, failures)`: the sheets that converted, and `(label, reason)` for each
    that did not."""
    pushed = []
    failures = []
    for assignment in assignments:
        entry, room = assignment[0], assignment[1]
        rooms = assignment[2] if len(assignment) > 2 else None
        try:
            pushed.append(read_sheet(doc, entry, room, phase_name, rooms, type_code_parameter))
        except Exception as e:
            failures.append((entry["label"], str(e)))
    return pushed, failures

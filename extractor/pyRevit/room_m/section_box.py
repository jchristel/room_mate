"""A 3D view's section box, as heights above the level of the room it was made for. No Revit in it.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Tested on CPython
(`extractor/tests/test_section_box.py`).

duHast exports a 3D view's section box in the model's own frame (`section_box_min`, `section_box_max`, mm). A
height in one model means nothing in another room's level, so the extractor turns the box's z range into
heights ABOVE THE LEVEL of the room the sheet was made for (`attach`, when a sheet is read), and the sheet
writer puts the new view's box that far above the level of ITS room (`above_floor_ft`). The level's z is the
room's own (`room_m.utils.rooms.level_z_ft`), never `Level.Elevation`, which is in another frame on some models.
"""

KEY = "section_z_above_floor_mm"
THREE_D = "three d"


def above_floor(view, level_z_mm):
    """`{"min", "max"}` mm above the level for one exported 3D view, or None when its section box is off or was
    not exported."""
    if not view or not view.get("section_box_active"):
        return None
    low, high = view.get("section_box_min"), view.get("section_box_max")
    try:
        z_min = float(low["z"]) - level_z_mm
        z_max = float(high["z"]) - level_z_mm
    except (KeyError, TypeError, ValueError):
        return None
    if z_max <= z_min:
        return None
    return {"min": z_min, "max": z_max}


def attach(sheet, level_z_mm):
    """Add `section_z_above_floor_mm` to the view of every 3D viewport of the exported `sheet` (a dict) that
    has a section box. Returns how many were given one."""
    count = 0
    for viewport in (sheet or {}).get("view_ports") or []:
        if (viewport.get("vp_type") or "").lower() != THREE_D:
            continue
        view = viewport.get("view") or {}
        heights = above_floor(view, level_z_mm)
        if heights is not None:
            view[KEY] = heights
            count += 1
    return count


def above_floor_ft(viewport, mm_to_ft):
    """`(min, max)` feet above the level for a template 3D viewport, or None when it carries none."""
    heights = ((viewport or {}).get("view") or {}).get(KEY)
    try:
        return mm_to_ft(heights["min"]), mm_to_ft(heights["max"])
    except (KeyError, TypeError):
        return None

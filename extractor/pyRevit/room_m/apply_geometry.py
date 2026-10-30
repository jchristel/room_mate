"""The arithmetic of applying a task, with no Revit in it.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so
that CPython runs it too (`extractor/tests/test_apply_geometry.py`): every rule about WHERE a
thing goes lives here, and `room_m.rls_write` and `room_m.apply_write` only turn the answers into Revit calls.

**Frames.** A task's positions are millimetres in the target model's own coordinates (the
server has already put them back through the inverse of the model's placement), and Revit's
internal unit is the foot. An item's `facing` is a unit vector in plan; its LEFT is the facing
turned a quarter turn counter-clockwise. A tag rule is in paper millimetres in the element's
own frame (x along its facing, y to its left), so a tag's place on this sheet is the element's
point plus the rule's offset times the view scale, turned with the facing. See
`src/tag_rules.rs`.
"""

import math

MM_PER_FT = 304.8

# The viewport type a plan is exported as.
PLAN = "floor plan"


def mm_to_ft(mm):
    return mm / MM_PER_FT


def facing_angle(item):
    """The item's facing as an angle in radians, counter-clockwise from +X. A zero vector faces
    +X: an item with no facing is not rotated."""
    fx = item.get("facing_x") or 0.0
    fy = item.get("facing_y") or 0.0
    if fx == 0.0 and fy == 0.0:
        return 0.0
    return math.atan2(fy, fx)


def to_model(item, offset, scale):
    """A rule's `offset` (`{x, y}` paper mm, element frame) as a model point in mm, for `item`
    drawn at `scale` (50 for 1:50)."""
    angle = facing_angle(item)
    c, s = math.cos(angle), math.sin(angle)
    ox = offset["x"] * scale
    oy = offset["y"] * scale
    return (
        item["x_mm"] + ox * c - oy * s,
        item["y_mm"] + ox * s + oy * c,
    )


def tag_placement(item, rule, scale):
    """Where to put the tag a `rule` describes on `item`, at view `scale`.

    Returns `{head, elbow, end, rotation, has_leader}`: points are `(x_mm, y_mm)` in the model
    (`elbow` and `end` are None when the rule has none), `rotation` is the tag's own angle in
    radians in the view, and `has_leader` is whether it draws one. None when `scale` is not a
    usable number: a tag at an unknown scale is a guess."""
    if not scale or scale <= 0:
        return None
    p = rule["placement"]
    elbow = p.get("elbow")
    end = p.get("end")
    return {
        "head": to_model(item, p["head"], scale),
        "elbow": to_model(item, elbow, scale) if elbow else None,
        "end": to_model(item, end, scale) if end else None,
        "rotation": facing_angle(item) + (p.get("rotation") or 0.0),
        "has_leader": bool(p.get("has_leader")),
    }


def _norm(s):
    return (s or "").strip().lower()


def room_rules(rules, viewport_type):
    """The room tag rules for a viewport of `viewport_type`. A room has no family or type, so
    these are matched by the viewport type alone (the server only carries rules of the task's
    viewport types, and a room tag rule is keyed by the category `Rooms`)."""
    return [
        r for r in rules
        if (r.get("tag_kind") or "") == "room" and _norm(r["key"]["viewport_type"]) == _norm(viewport_type)
    ]


def room_like(x_mm, y_mm):
    """A room as `tag_placement` wants an element: a point, and no orientation of its own, so its
    tag's offset is in the plan's own axes (+x, then +y), as the server learned it."""
    return {"x_mm": x_mm, "y_mm": y_mm, "facing_x": 1.0, "facing_y": 0.0}


def sheet_size_of(sheet):
    """The size the layout asks for, as duHast exported it; None when it did not say."""
    body = sheet.get("sheet") or {}
    size = body.get("sheet_size")
    return size if isinstance(size, (str, type(u""))) and size.strip() else None


def viewports_of(sheet):
    """The viewports of a task sheet, in the order exported."""
    return list((sheet.get("sheet") or {}).get("view_ports") or [])


def viewport_scale(viewport):
    """The view scale the viewport was exported at, or None."""
    view = viewport.get("view") or {}
    scale = view.get("view_scale")
    return scale if isinstance(scale, (int, float)) and scale > 0 else None


def viewport_centre_ft(viewport):
    """Where the viewport's centre sits on the sheet, in feet, or None.

    duHast exports `centre_point` through `convert_XYZ_to_point2`, which is millimetres."""
    c = viewport.get("centre_point")
    if not c:
        return None
    x = c.get("x")
    y = c.get("y")
    if x is None or y is None:
        return None
    return (mm_to_ft(x), mm_to_ft(y))


def crop_size_mm(viewport, room_width_mm=0.0, room_height_mm=0.0):
    """The crop box a made view takes: the size the source view had, but never smaller than the
    target room's own extent (a bigger room must not be cut off by a small room's crop).
    `(width, height)` in mm, or None when the export carried no crop."""
    box = (viewport.get("view") or {}).get("bounding_box") or {}
    extent = _extent(box)
    if extent is None:
        return None
    w, h = extent
    return (max(w, room_width_mm), max(h, room_height_mm))


def _extent(box):
    """`(width, height)` of an exported 2D box, whichever way duHast spelt its corners
    (`min`/`max` points, or `min_x`/`max_x` fields); None when it holds neither."""
    lo = box.get("min") or box.get("min_point")
    hi = box.get("max") or box.get("max_point")
    if lo and hi:
        return (abs(hi["x"] - lo["x"]), abs(hi["y"] - lo["y"]))
    try:
        return (abs(box["max_x"] - box["min_x"]), abs(box["max_y"] - box["min_y"]))
    except (KeyError, TypeError):
        return None

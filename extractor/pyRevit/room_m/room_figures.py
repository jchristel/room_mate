"""The figures of the room (or rooms) a sheet covers: size, proportions, area and item count.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

Worked out in Revit when a sheet is exported and sent with it (`RoomSummary` on the server), so a
sheet needs no room to be linked to it: the library ranks a sheet on these figures alone. The pure
half (`min_rectangle`, `combine`) is tested on CPython (`extractor/tests/test_room_figures.py`); the
Revit half (`read_room`) only collects what duHast offers.

**Several rooms on one sheet** are one figure: the rectangle is the smallest one enclosing every
room's outline, the area and the item count are sums.
"""

import math

SQ_MM_PER_SQ_M = 1000000.0


def _cross(o, a, b):
    return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])


def convex_hull(points):
    """The convex hull of `points` (x, y), counter-clockwise, without repeats."""
    pts = sorted(set((float(x), float(y)) for x, y in points))
    if len(pts) <= 2:
        return pts
    lower = []
    for p in pts:
        while len(lower) >= 2 and _cross(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    upper = []
    for p in reversed(pts):
        while len(upper) >= 2 and _cross(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    return lower[:-1] + upper[:-1]


def min_rectangle(points):
    """`(long_side, short_side)` of the smallest rectangle enclosing `points`, or None for fewer
    than three distinct points. Whatever way the room is turned, it reads the same."""
    hull = convex_hull(points)
    if len(hull) < 3:
        return None
    best = None
    for i in range(len(hull)):
        a = hull[i]
        b = hull[(i + 1) % len(hull)]
        length = math.hypot(b[0] - a[0], b[1] - a[1])
        if length == 0:
            continue
        ux, uy = (b[0] - a[0]) / length, (b[1] - a[1]) / length
        along = [(p[0] - a[0]) * ux + (p[1] - a[1]) * uy for p in hull]
        across = [-(p[0] - a[0]) * uy + (p[1] - a[1]) * ux for p in hull]
        w = max(along) - min(along)
        h = max(across) - min(across)
        if best is None or w * h < best[0]:
            best = (w * h, max(w, h), min(w, h))
    return (best[1], best[2]) if best else None


def polygon_area(points):
    """The area of a simple polygon, in the points' units squared."""
    total = 0.0
    for i in range(len(points)):
        x1, y1 = points[i]
        x2, y2 = points[(i + 1) % len(points)]
        total += x1 * y2 - x2 * y1
    return abs(total) / 2.0


def combine(rooms):
    """The figure of `rooms`, each `{"outline": [(x_mm, y_mm)], "items": int}`: the `room_summary`
    the server stores, or None when no room has an outline."""
    points = []
    area = 0.0
    items = 0
    for room in rooms:
        outline = room.get("outline") or []
        if len(outline) < 3:
            continue
        points.extend(outline)
        area += polygon_area(outline)
        items += int(room.get("items") or 0)
    rect = min_rectangle(points)
    if rect is None:
        return None
    return {
        "x_mm": round(rect[0], 1),
        "y_mm": round(rect[1], 1),
        "area_m2": round(area / SQ_MM_PER_SQ_M, 3),
        "items": items,
    }


def room_label(room):
    """"number name" for the library's list."""
    return "{} {}".format(room.get("number") or "", room.get("name") or "").strip()


def read_room(doc, entry):
    """`{"outline", "items"}` for one entry of `room_m.utils.sheets.placed_rooms`, read from
    Revit; the room's own phase decides which items are in it."""
    from duHast.Revit.Family.Export.to_data_item import DEFAULT_ITEM_CATEGORIES
    from duHast.Revit.Family.family_instance_facts import count_instances_in_room
    from duHast.Revit.Rooms.rooms import get_room_outline_mm, get_room_phase

    room = entry["element"]
    phase = get_room_phase(doc, room)
    items = 0
    if phase is not None:
        items = count_instances_in_room(doc, int(entry["id"]), phase, DEFAULT_ITEM_CATEGORIES)
    return {"outline": get_room_outline_mm(room), "items": items}

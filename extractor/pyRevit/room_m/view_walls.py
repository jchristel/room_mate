"""Which walls stand between a 3D view's camera and the middle of its room. No Revit in it.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Tested on CPython
(`extractor/tests/test_view_walls.py`).

A new 3D view looks at a room from outside it, so the room's own walls on the camera side hide what is in it.
Seen from above, a wall is in the way when the line from the camera to the middle of the room crosses it. The
test is made in plan, in feet or any unit (it is only a crossing), and ignores heights: a wall the line of
sight passes over is hidden too, which is the cheaper mistake (the room reads the same with a low wall gone).

**The line alone is not enough at a corner** (2026-11-07, `docs/3D wall hiding logic.pdf` in the main checkout): a
camera off a corner looks over two walls, and its line to the middle crosses only one; the other still stands
across the near edge of the view. So `walls_to_hide` also hides every wall of the room's OUTER loop that faces
the camera, the camera being on its outside (`faces`): seen from a corner both walls of that corner go, seen
square on only the near wall does, and a wall the camera looks along stays. A hole's walls (a column) are hidden
only when the line crosses them.

**What stands along a hidden wall is seen through, not hidden** (`items_to_see_through`): the task carries the
room's items along its walls (`rls.wall_items`, worked out on the server from the FF&E it holds), and the plug-in
keeps those hosted by, or within the task's band of, a wall it hides.
"""

EPSILON = 1.0e-9


def _cross(o, a, b):
    return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])


def _on_segment(a, b, p):
    return (
        min(a[0], b[0]) - EPSILON <= p[0] <= max(a[0], b[0]) + EPSILON
        and min(a[1], b[1]) - EPSILON <= p[1] <= max(a[1], b[1]) + EPSILON
    )


def segments_cross(p1, p2, q1, q2):
    """True when segment p1-p2 meets segment q1-q2 (touching counts: a line through a corner meets both walls)."""
    d1, d2 = _cross(q1, q2, p1), _cross(q1, q2, p2)
    d3, d4 = _cross(p1, p2, q1), _cross(p1, p2, q2)
    if ((d1 > EPSILON and d2 < -EPSILON) or (d1 < -EPSILON and d2 > EPSILON)) and (
        (d3 > EPSILON and d4 < -EPSILON) or (d3 < -EPSILON and d4 > EPSILON)
    ):
        return True
    if abs(d1) <= EPSILON and _on_segment(q1, q2, p1):
        return True
    if abs(d2) <= EPSILON and _on_segment(q1, q2, p2):
        return True
    if abs(d3) <= EPSILON and _on_segment(p1, p2, q1):
        return True
    return abs(d4) <= EPSILON and _on_segment(p1, p2, q2)


def eye_xy(centre_xy, forward, distance):
    """Where the camera is in plan: back from the centre along the view's direction. None when the view looks
    straight down (no plan direction, so no wall is between)."""
    length = (forward[0] ** 2 + forward[1] ** 2) ** 0.5
    if length < 1.0e-6:
        return None
    k = distance / length
    return (centre_xy[0] - forward[0] * k, centre_xy[1] - forward[1] * k)


def blocking(eye, centre, boundary):
    """The keys, in order and once each, of the boundary pieces the line from `eye` to `centre` crosses.
    `boundary` is a list of `(key, (x0, y0), (x1, y1))`, one per piece of the room's boundary (a wall can have
    several pieces). `eye` None gives none."""
    if eye is None:
        return []
    found = []
    for key, a, b in boundary:
        if key not in found and segments_cross(eye, centre, a, b):
            found.append(key)
    return found


# A wall faces the camera when the camera is outside its line by at least this share of the distance between
# them (about 3 degrees): a wall the camera looks along stays.
FACING = 0.05


def signed_area(loop):
    """Twice the signed area of a loop of `(key, a, b)` pieces, from their start points: positive when it runs
    counter-clockwise."""
    total = 0.0
    for _key, a, b in loop:
        total += a[0] * b[1] - b[0] * a[1]
    return total


def faces(eye, a, b, counter_clockwise):
    """True when the camera at `eye` is on the outside of the outer-loop piece `a`-`b`, by `FACING`. Outside is
    to the right of a counter-clockwise loop's direction of travel."""
    dx, dy = b[0] - a[0], b[1] - a[1]
    length = (dx * dx + dy * dy) ** 0.5
    if length < EPSILON:
        return False
    nx, ny = (dy / length, -dx / length) if counter_clockwise else (-dy / length, dx / length)
    mx, my = (a[0] + b[0]) / 2.0 - eye[0], (a[1] + b[1]) / 2.0 - eye[1]
    distance = (mx * mx + my * my) ** 0.5
    return -(nx * mx + ny * my) > FACING * distance


def walls_to_hide(eye, centre, loops):
    """The keys, in order and once each, of the walls to hide: every piece the line from `eye` to `centre` crosses
    (`blocking`), and every piece of the first loop (the room's outer boundary, as Revit lists it) that faces the
    camera. `loops` is a list of loops of `(key, (x0, y0), (x1, y1))` in boundary order; a piece that is not a wall
    has the key None, so it still counts for the loop's direction and is never hidden. `eye` None gives none."""
    if eye is None:
        return []
    found = []
    for index, loop in enumerate(loops):
        counter_clockwise = signed_area(loop) > 0
        for key, a, b in loop:
            if key is None or key in found:
                continue
            if segments_cross(eye, centre, a, b) or (index == 0 and faces(eye, a, b, counter_clockwise)):
                found.append(key)
    return found


def _to_segment(p, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    length2 = dx * dx + dy * dy
    t = 0.0 if length2 < EPSILON else max(0.0, min(1.0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2))
    fx, fy = a[0] + dx * t, a[1] + dy * t
    return ((p[0] - fx) ** 2 + (p[1] - fy) ** 2) ** 0.5


def items_to_see_through(wall_items, hidden, pieces):
    """The ids, in the task's order, of the items to make transparent beside the walls being hidden.

    `wall_items` is the task's `rls.wall_items` (`{"band_mm", "items": [{"id", "footprint", "host_wall"}]}`, the
    server's list of the room's items along any of its walls, footprints in mm); `hidden` the keys
    `walls_to_hide` returned and `pieces` every `(key, (x0, y0), (x1, y1))` of the room's boundary, in MM. An item
    goes when it is hosted by a hidden wall or a corner of its footprint is within the band of a hidden piece:
    which walls are hidden is decided here, from Revit's boundary, and the server only says what stands along them."""
    if not wall_items or not hidden:
        return []
    band = float(wall_items.get("band_mm") or 0.0)
    gone = set(str(k) for k in hidden)
    near = [(a, b) for key, a, b in pieces if key is not None and str(key) in gone]
    out = []
    for item in wall_items.get("items") or []:
        host = item.get("host_wall")
        if host is not None and str(host) in gone:
            out.append(item["id"])
            continue
        if any(_to_segment(p, a, b) <= band for p in item.get("footprint") or [] for a, b in near):
            out.append(item["id"])
    return out

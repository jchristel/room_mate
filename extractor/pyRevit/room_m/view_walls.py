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

**The server decides it now** (2026-11-08, `service::rls_three_d`): a task carries, per 3D view, the edges of the
room's outline whose walls go and the items to see through, and the plug-in hides every wall with a face along an
edge (`walls_along`). `walls_to_hide` is what a task made before that (or for a room the server has no outline of)
still uses.
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



# A wall's face is along an edge when it runs within about 10 degrees of it and shares at least this much of it:
# the wall of the side next to a hidden one meets it at the corner, end on, and must stay.
PARALLEL_SIN = 0.17
OVERLAP_MIN_MM = 100.0


def _along(face, edge, band):
    (a, b), (p, q) = face, edge
    ex, ey = q[0] - p[0], q[1] - p[1]
    length = (ex * ex + ey * ey) ** 0.5
    fx, fy = b[0] - a[0], b[1] - a[1]
    flen = (fx * fx + fy * fy) ** 0.5
    if length < EPSILON or flen < EPSILON:
        return False
    tx, ty = ex / length, ey / length
    if abs(tx * fy - ty * fx) / flen > PARALLEL_SIN:
        return False
    # how far the face is off the edge's line, and how much of the edge it covers
    off = [abs(tx * (pt[1] - p[1]) - ty * (pt[0] - p[0])) for pt in (a, b)]
    if sum(off) / 2.0 > band:
        return False
    t0 = (a[0] - p[0]) * tx + (a[1] - p[1]) * ty
    t1 = (b[0] - p[0]) * tx + (b[1] - p[1]) * ty
    overlap = min(length, max(t0, t1)) - max(0.0, min(t0, t1))
    return overlap >= OVERLAP_MIN_MM


def faces_of(a, b, half_width):
    """The two faces of a straight wall whose location line runs `a`-`b`, `half_width` either side."""
    dx, dy = b[0] - a[0], b[1] - a[1]
    length = (dx * dx + dy * dy) ** 0.5
    if length < EPSILON:
        return []
    nx, ny = -dy / length * half_width, dx / length * half_width
    return [((a[0] + nx, a[1] + ny), (b[0] + nx, b[1] + ny)), ((a[0] - nx, a[1] - ny), (b[0] - nx, b[1] - ny))]


def walls_along(walls, edges, band):
    """The keys, in order, of the walls with a face along one of the server's hidden `edges` (the task's
    `rls.three_ds[n].hide`: `[[x0, y0], [x1, y1]]` in mm), within `band` mm of it. `walls` is a list of
    `(key, (x0, y0), (x1, y1), half_width)`, the straight walls' location lines, in mm. A wall that only meets an
    edge at its end (the side wall at the corner) is not along it."""
    out = []
    for key, a, b, half_width in walls:
        if key in out:
            continue
        if any(_along(face, (tuple(e[0]), tuple(e[1])), band) for face in faces_of(a, b, half_width) for e in edges):
            out.append(key)
    return out

"""Which walls stand between a 3D view's camera and the middle of its room. No Revit in it.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Tested on CPython
(`extractor/tests/test_view_walls.py`).

A new 3D view looks at a room from outside it, so the room's own walls on the camera side hide what is in it.
Seen from above, a wall is in the way when the line from the camera to the middle of the room crosses it. The
test is made in plan, in feet or any unit (it is only a crossing), and ignores heights: a wall the line of
sight passes over is hidden too, which is the cheaper mistake (the room reads the same with a low wall gone).
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

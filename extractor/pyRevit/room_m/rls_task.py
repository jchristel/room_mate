"""Plan a room layout sheet (RLS) task against the model's RLS settings. No Revit in it.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so
that CPython runs it too (`extractor/tests/test_rls_task.py`): what Revit is asked is behind a
`facts` object, and what the answers MEAN is decided here.

**RoomMate decides, Revit applies** (the RLS methodology). The task already says the sheet size, where
each elevation marker and its views go in this room and which items are tagged where
(`rls` in the task, built by the server). This module adds what only the model can say: the title
block family for that size, the view types and the base schedule, the room tag and the tag family,
and the number and names, worked out from the room's own parameters by the templates in the model's
settings (`rls_settings`). Nothing here guesses a convention: a template or a mapping that is missing
is an abort that says which.

**The unhappy path, as the methodology states it.** Aborts (nothing is written, the task is
reported aborted with the reason): the sheet already exists, a view already exists, no title block
family for the size, a view type or the base schedule that is not in the model, a template that
cannot be filled from the room, the room or the phase is missing, the settings are incomplete.
Warnings (written, reported as completed with warnings): no room tag defined, no independent tag
defined, elevations the server could not place.

`facts` answers, for the model open in Revit:
  phase_exists(phase_name) -> bool
  find_room(number, room_id) -> a room handle or None
  room_values(room) -> {parameter name: text} for the naming templates
  title_block_exists(name) -> bool          ("Family : Type")
  view_type_exists(kind, name) -> bool      (kind "plan" or "elevation")
  schedule_exists(name) -> bool
  tag_type_exists(kind, name) -> bool       (kind "room" or "independent")
  sheet_exists(number) -> bool
  views_in_use(names) -> [names that are the name of a view already]
"""

import re

from room_m import apply_geometry as geo
from room_m import rls_settings

CREATE_RLS = "create_rls"

SHEET_EXISTS = "sheet_exists"
VIEW_EXISTS = "view_exists"
NO_TITLE_BLOCK = "no_title_block"
NO_SIZE = "no_sheet_size"
SETTINGS_INCOMPLETE = "settings_incomplete"
VIEW_TYPE_MISSING = "view_type_missing"
SCHEDULE_MISSING = "schedule_missing"
TEMPLATE_UNRESOLVED = "template_unresolved"
ROOM_MISSING = "room_missing"
PHASE_MISSING = "phase_missing"
UNKNOWN_KIND = "unknown_kind"
NO_ROOM_TAG = "no_room_tag"
NO_INDEPENDENT_TAG = "no_independent_tag"
ELEVATION_NOT_PLACED = "elevation_not_placed"

_TOKEN = re.compile(r"\{([^{}]*)\}")
_BAD_NUMBER_CHARS = re.compile(r"[\\/:*?\"<>|{}\[\];~`]")
DEFAULT_SCALE = 50


def finding(code, message):
    return {"code": code, "message": message}


def render(template, values):
    """`(text, missing)`: `template` with each `{Name}` replaced by the room's value of that
    parameter (case ignored), and the names that the room has no value for. A name the room does
    not have is never replaced by nothing: the caller refuses."""
    lowered = dict((str(k).strip().lower(), v) for k, v in (values or {}).items())
    missing = []

    def replace(m):
        key = m.group(1).strip().lower()
        if key in lowered:
            return "" if lowered[key] is None else str(lowered[key])
        missing.append(m.group(1))
        return ""

    return _TOKEN.sub(replace, template or ""), missing


def clean_number(text):
    """A sheet number Revit accepts: its refused characters become a dash."""
    return _BAD_NUMBER_CHARS.sub("-", (text or "").strip())


def _size_of(task):
    rls = task.get("rls") or {}
    size = rls.get("sheet_size")
    if size:
        return str(size).strip().lower()
    for sheet in task.get("sheets") or []:
        size = geo.sheet_size_of(sheet)
        if size:
            return str(size).strip().lower()
    return None


def _template_sheet(task):
    sheets = task.get("sheets") or []
    return sheets[0] if sheets else {}


def _template_viewport(sheet, view_id):
    for vp in geo.viewports_of(sheet):
        if vp.get("view_id") == view_id:
            return vp
    return None


def _first_of_type(sheet, vp_type):
    for vp in geo.viewports_of(sheet):
        if (vp.get("vp_type") or "").lower() == vp_type:
            return vp
    return None


def _elevations(task, template, base):
    """The elevation views to make, in the order they are named: marker by marker, by index."""
    out = []
    n = 0
    for marker in (task.get("rls") or {}).get("markers") or []:
        for view in sorted(marker.get("views") or [], key=lambda v: v.get("marker_index", 0)):
            n += 1
            vp = _template_viewport(template, view.get("view_id"))
            out.append({
                "name": "{} ELEV {}".format(base, n),
                "marker_id": marker.get("marker_id"),
                "marker_location": marker.get("location"),
                "marker_index": view.get("marker_index", 0),
                "view_id": view.get("view_id"),
                "line_start": view.get("line_start"),
                "line_end": view.get("line_end"),
                "looks": view.get("looks"),
                "depth_mm": view.get("depth_mm"),
                "faces_edge": view.get("faces_edge"),
                "viewport": vp,
                "scale": geo.viewport_scale(vp) if vp else None,
            })
    return out


def _plan_tags(task):
    """The tags the server worked out for the plan view: `[tag]`, each with an item id and a head."""
    tags = (task.get("rls") or {}).get("tags") or {}
    for view in tags.get("views") or []:
        if view.get("view_type") == geo.PLAN:
            return [t for t in view.get("tagged") or [] if t.get("head")]
    return []


def build(task, settings, values):
    """`(plan, aborts)`: what to make for `task`, and what stops it. The plan is plain data (names,
    sizes, template viewports, markers, tags), so the writer only has to do it."""
    aborts = []
    template = _template_sheet(task)
    size = _size_of(task)
    if not size:
        aborts.append(finding(NO_SIZE, "no sheet size: the task names none and its template sheet has none"))
        title_block = None
    else:
        title_block = (settings.get("sheet_sizes") or {}).get(size)
        if not title_block:
            aborts.append(finding(NO_TITLE_BLOCK, "the settings map no title block to size {}".format(size.upper())))

    rendered = {}
    for key, label in (("sheet_number", "sheet number"), ("sheet_name", "sheet name"), ("view_name", "view name")):
        text, missing = render(settings.get(key), values)
        if missing:
            aborts.append(finding(
                TEMPLATE_UNRESOLVED,
                "the {} logic uses {} which the room does not have".format(
                    label, ", ".join("{" + m + "}" for m in missing))))
        rendered[key] = text.strip()
    number = clean_number(rendered["sheet_number"])
    if not number:
        aborts.append(finding(TEMPLATE_UNRESOLVED, "the sheet number logic gives an empty number for this room"))
    base = rendered["view_name"]
    if not base:
        aborts.append(finding(TEMPLATE_UNRESOLVED, "the view name logic gives an empty name for this room"))

    plan_vp = _first_of_type(template, geo.PLAN)
    schedule_vp = _first_of_type(template, "schedule")
    elevations = _elevations(task, template, base)
    plan = {
        "number": number,
        "name": rendered["sheet_name"],
        "size": size,
        "title_block": title_block,
        "base": base,
        "plan_name": "{} PLAN".format(base),
        "plan_viewport": plan_vp,
        "plan_scale": (geo.viewport_scale(plan_vp) if plan_vp else None) or DEFAULT_SCALE,
        "elevations": elevations,
        "schedule_name": "{} SCHEDULE".format(base),
        "schedule_viewport": schedule_vp,
        "plan_view_type": settings.get("plan_view_type"),
        "elevation_view_type": settings.get("elevation_view_type"),
        "schedule_base": settings.get("schedule_base"),
        "room_tag": settings.get("room_tag") or None,
        "independent_tag": settings.get("independent_tag") or None,
        "tags": _plan_tags(task),
    }
    return plan, aborts


def check(task, settings, facts, taken):
    """`(plan, aborts, warnings)` for one task. `taken` holds the sheet numbers and view names the
    tasks before this one will make, so two tasks of one list cannot collide with each other."""
    aborts, warnings = [], []
    label = (task.get("room") or {}).get("number") or task.get("id") or "?"
    if task.get("kind") != CREATE_RLS:
        return None, [finding(UNKNOWN_KIND, "{}: this script makes room layout sheets, not {} tasks".format(label, task.get("kind")))], []
    missing = rls_settings.missing(settings)
    if missing:
        return None, [finding(SETTINGS_INCOMPLETE, "the RLS settings are incomplete: {}".format(", ".join(missing)))], []
    if not facts.phase_exists(task.get("phase")):
        aborts.append(finding(PHASE_MISSING, "{}: the model has no phase '{}'".format(label, task.get("phase"))))
    room = facts.find_room((task.get("room") or {}).get("number"), (task.get("room") or {}).get("room_id"))
    if room is None:
        return None, aborts + [finding(ROOM_MISSING, "{}: the room is no longer in the model".format(label))], []

    plan, built = build(task, settings, facts.room_values(room))
    aborts.extend(built)
    plan["room"] = room
    if aborts:
        return plan, aborts, []

    if not facts.title_block_exists(plan["title_block"]):
        aborts.append(finding(NO_TITLE_BLOCK, "{}: the model has no title block '{}' (size {})".format(
            label, plan["title_block"], plan["size"].upper())))
    for kind, name, what in (
        ("plan", plan["plan_view_type"], "plan view type"),
        ("elevation", plan["elevation_view_type"], "elevation view type"),
    ):
        if not facts.view_type_exists(kind, name):
            aborts.append(finding(VIEW_TYPE_MISSING, "{}: the {} '{}' is not in the model".format(label, what, name)))
    if not facts.schedule_exists(plan["schedule_base"]):
        aborts.append(finding(SCHEDULE_MISSING, "{}: the base schedule '{}' is not in the model".format(label, plan["schedule_base"])))

    number = plan["number"]
    if number in taken["sheets"] or facts.sheet_exists(number):
        aborts.append(finding(SHEET_EXISTS, "{}: sheet {} already exists".format(label, number)))
    names = [plan["plan_name"], plan["schedule_name"]] + [e["name"] for e in plan["elevations"]]
    in_use = list(facts.views_in_use(names)) + [n for n in names if n.strip().lower() in taken["views"]]
    if in_use:
        aborts.append(finding(VIEW_EXISTS, "{}: a view named {} already exists".format(
            label, ", ".join("'{}'".format(n) for n in sorted(set(in_use))))))

    if plan["room_tag"] is None or not facts.tag_type_exists("room", plan["room_tag"]):
        warnings.append(finding(NO_ROOM_TAG, "{}: no room tag is defined (or it is not in the model): the room is not tagged".format(label)))
    if plan["independent_tag"] is None or not facts.tag_type_exists("independent", plan["independent_tag"]):
        warnings.append(finding(NO_INDEPENDENT_TAG, "{}: no independent tag is defined (or it is not in the model): no item is tagged".format(label)))
    for f in task.get("findings") or []:
        if f.get("kind") == "elevation_not_placed":
            warnings.append(finding(ELEVATION_NOT_PLACED, "{}: {}".format(label, f.get("message"))))

    if not aborts:
        taken["sheets"].add(number)
        taken["views"].update(n.strip().lower() for n in names)
    return plan, aborts, warnings


def plan_tasks(tasks, settings, facts):
    """Check every task. Returns the verdicts in the order given, in the shape `room_m.task_plan`
    and `room_m.apply_run` use: `{"task", "aborts", "warnings", "plan"}`."""
    taken = {"sheets": set(), "views": set()}
    verdicts = []
    for task in tasks:
        plan, aborts, warnings = check(task, settings, facts, taken)
        verdicts.append({"task": task, "aborts": aborts, "warnings": warnings, "plan": plan})
    return verdicts

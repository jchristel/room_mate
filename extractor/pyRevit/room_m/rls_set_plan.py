"""Plan a SET task (`create_rls_set`, multi-room RLS) against the model's RLS settings. No Revit in it.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so that CPython runs it
too (`extractor/tests/test_rls_set_plan.py`); what Revit is asked is behind `facts`, as in `room_m.rls_task`, whose
facts, findings and template filling this reuses.

**The server laid the set out; this adds what only the model can say.** The task (`service::rls_set_task`) carries the
rooms (the lead first), the paper size, the plan crop, each room's marker and elevations, the schedules, whether there
is a 3D view, and every viewport's sheet and centre on the paper. This module finds the rooms in the model, names
everything and checks that it can be made. The naming rules are the plan's ("Naming a set"):

- The sheet number and name, the plan and the 3D view (names and Title on Sheet) use the SET templates
  (`rls_settings.SET_TEMPLATES`, empty = the single-room one), filled from the LEAD room. `{Rooms:Parameter}` is that
  parameter of every room, distinct values joined by ", " (lead first); a room without it aborts.
  `{Sheet_Counter}` is the sheet's place in the set, from 1, and the sheet number must name it when the set has more
  than one sheet (else every sheet would get one number).
- Each room's elevations and its schedule keep the single-room templates, filled from THAT room, `{Seq_Counter}`
  counting 1 to 4 within it, so `{Number}` keeps them apart.

Aborts are `rls_task`'s: a room or its phase missing, a template the rooms cannot fill, no title block for the size, a
view type or the base schedule missing, a sheet or view name taken (in the model or twice in the set).
"""

from room_m import apply_geometry as geo
from room_m import detail_numbers
from room_m import rls_settings
from room_m import rls_task
from room_m.rls_task import (
    NO_TITLE_BLOCK,
    PHASE_MISSING,
    ROOM_MISSING,
    SCHEDULE_MISSING,
    SCHEDULE_NOT_ROOM_FILTERED,
    SETTINGS_INCOMPLETE,
    SHEET_EXISTS,
    TEMPLATE_UNRESOLVED,
    THREE_D_NO_DIRECTION,
    VIEW_EXISTS,
    VIEW_TYPE_MISSING,
    finding,
)
from room_m.rls_template import ROOMS_JOIN, SEQ_COUNTER, SHEET_COUNTER, TOKEN, rooms_parameter

CREATE_RLS_SET = "create_rls_set"
NO_PLAN_FOR_ELEVATIONS = "no_plan_for_elevations"
NO_ROOM_TAG = rls_task.NO_ROOM_TAG


def _lowered(values):
    return dict((str(k).strip().lower(), v) for k, v in (values or {}).items())


def render_set(template, lead, every, extra=None):
    """`(text, missing)`: `template` filled from the lead room's `lead` values, `{Rooms:P}` from every room's
    (`every`, a list of value dicts, lead first) and `extra` (`{Sheet_Counter: 2}`). `missing` names each word no room
    could fill: for `{Rooms:P}`, any room without `P` is enough, because a list that leaves a room out reads as the
    whole set."""
    missing = []
    rooms = [_lowered(v) for v in every or []]

    def rooms_of(m):
        name = rooms_parameter(m.group(1))
        if name is None:
            return m.group(0)
        key = name.strip().lower()
        seen = []
        for values in rooms:
            if key not in values or values[key] is None or str(values[key]).strip() == "":
                missing.append(m.group(1))
                return ""
            text = str(values[key]).strip()
            if text not in seen:
                seen.append(text)
        return ROOMS_JOIN.join(seen)

    first = TOKEN.sub(rooms_of, template or "")
    filled = dict(lead or {})
    filled.update(extra or {})
    text, rest = rls_task.render(first, filled)
    return text.strip(), missing + rest


def _template_ports(sheet, vp_type):
    return [vp for vp in geo.viewports_of(sheet) if (vp.get("vp_type") or "").lower() == vp_type]


def _first(sheet, vp_type):
    ports = _template_ports(sheet, vp_type)
    return ports[0] if ports else None


def _sheet_of_views(set_plan):
    """`{(kind, room_id, marker_index): (sheet index, centre_mm)}` for every viewport the task places."""
    out = {}
    for i, sheet in enumerate(set_plan.get("sheets") or []):
        for v in sheet.get("views") or []:
            out[(v.get("kind"), v.get("room_id"), v.get("marker_index"))] = (i, v.get("centre_mm"), v.get("size_mm"))
    return out


class _Namer(object):
    """Fills templates and collects what cannot be filled, once per (what, room): the abort says which template and
    which room, so a person knows which parameter to fill in."""

    def __init__(self, aborts):
        self.aborts = aborts
        self._said = set()

    def fill(self, label, template, lead, every, extra=None, room_label=None):
        if not template:
            self._once((label, None), "the settings have no {}".format(label))
            return ""
        text, missing = render_set(template, lead, every, extra)
        if missing:
            self._once((label, room_label), "the {}{} uses {} which {} not have".format(
                label, " of room {}".format(room_label) if room_label else "",
                ", ".join("{" + m + "}" for m in sorted(set(missing))),
                "a room of the set does" if any(rooms_parameter(m) for m in missing) else "the room does"))
        elif not text:
            self._once((label, room_label), "the {} gives an empty text{}".format(
                label, " for room {}".format(room_label) if room_label else ""))
        return text

    def _once(self, key, message):
        if key not in self._said:
            self._said.add(key)
            self.aborts.append(finding(TEMPLATE_UNRESOLVED, message))


def build(task, settings, rooms):
    """`(plan, aborts)` for a set task. `rooms` is `[(task room, handle, values)]` in the task's order (lead first).
    The plan is plain data the writer only has to do (`room_m.rls_set_write`)."""
    aborts = []
    namer = _Namer(aborts)
    s = rls_settings.with_view_names(settings)
    set_plan = task.get("rls_set") or {}
    template = rls_task._template_sheet(task)
    lead_values = rooms[0][2] if rooms else {}
    every = [values for _r, _h, values in rooms]
    label_of = dict(((r.get("room_id")), (r.get("number") or r.get("name") or r.get("room_id"))) for r, _h, _v in rooms)
    values_of = dict((r.get("room_id"), values) for r, _h, values in rooms)
    handle_of = dict((r.get("room_id"), handle) for r, handle, _v in rooms)
    where = _sheet_of_views(set_plan)

    size = str(set_plan.get("sheet_size") or "").strip().lower()
    title_block = (s.get("sheet_sizes") or {}).get(size)
    if not title_block:
        aborts.append(finding(NO_TITLE_BLOCK, "the settings map no title block to size {}".format(size.upper() or "?")))

    count = len(set_plan.get("sheets") or [])
    number_template = rls_settings.set_template(s, "set_sheet_number")
    if count > 1 and SHEET_COUNTER.lower() not in [t.strip().lower() for t in TOKEN.findall(number_template)]:
        aborts.append(finding(TEMPLATE_UNRESOLVED, "the set has {} sheets and its sheet number logic '{}' has no {{{}}}: "
                                                   "every sheet would get one number".format(count, number_template, SHEET_COUNTER)))
    sheets = []
    for i in range(count):
        counter = {SHEET_COUNTER: i + 1}
        number = rls_task.clean_number(namer.fill("set sheet number logic", number_template, lead_values, every, counter))
        name = namer.fill("set sheet name logic", rls_settings.set_template(s, "set_sheet_name"), lead_values, every, counter)
        sheets.append({"number": number, "name": name})

    def title(key, values, n, room_label=None, set_wide=False):
        template = rls_settings.set_template(s, key) if set_wide else (s.get(key) or "").strip()
        if not template:
            return None
        return namer.fill("title on sheet ({})".format(key), template, values, every if set_wide else [values],
                          {SEQ_COUNTER: n}, room_label) or None

    plan_vp = _first(template, geo.PLAN)
    elevation_vp = _first(template, "elevation")
    three_d_vp = _first(template, rls_task.THREE_D)

    plan_view = None
    if set_plan.get("plan"):
        crop = rls_task._plan_crop({"rls": {"plan_crop": (set_plan.get("plan") or {}).get("crop")}})
        at = where.get(("plan", None, None))
        plan_view = {
            "name": namer.fill("set plan view name logic", rls_settings.set_template(s, "set_view_name_plan"),
                               lead_values, every, {SEQ_COUNTER: 1}),
            "title": title("set_title_on_sheet_plan", lead_values, 1, set_wide=True),
            "scale": set_plan["plan"].get("scale") or rls_task.DEFAULT_SCALE,
            "crop": crop,
            "viewport": plan_vp,
            "sheet": at[0] if at else None,
            "centre_mm": at[1] if at else None,
        }

    elevations = []
    for marker in set_plan.get("markers") or []:
        room_id = marker.get("room_id")
        values = values_of.get(room_id) or {}
        views = sorted(marker.get("views") or [], key=lambda v: v.get("marker_index", 0))
        for n, view in enumerate(views, 1):
            at = where.get(("elevation", room_id, view.get("marker_index")))
            elevations.append({
                "room_id": room_id,
                "room": handle_of.get(room_id),
                "name": namer.fill("elevation view name logic", (s.get("view_name_elevation") or "").strip(),
                                   values, [values], {SEQ_COUNTER: n}, label_of.get(room_id)),
                "title": title("title_on_sheet_elevation", values, n, label_of.get(room_id)),
                # the writer keys its new views by this, as it keys a single-room task's by the template's view id
                "view_id": "{}:{}".format(room_id, view.get("marker_index")),
                "marker_id": room_id,
                "marker_location": marker.get("location"),
                "marker_index": view.get("marker_index", 0),
                "line_start": view.get("line_start"),
                "line_end": view.get("line_end"),
                "looks": view.get("looks"),
                "depth_mm": view.get("depth_mm"),
                "height_mm": view.get("height_mm"),
                "scale": view.get("scale"),
                "viewport": elevation_vp,
                "sheet": at[0] if at else None,
                "centre_mm": at[1] if at else None,
            })

    three_ds, skipped = [], False
    if set_plan.get("three_d"):
        view = (three_d_vp or {}).get("view") or {}
        forward = rls_task._direction(view.get("forward_direction"))
        up = rls_task._direction(view.get("up_direction"))
        at = where.get(("three_d", None, None))
        if forward is None or up is None:
            skipped = True
        else:
            turn = plan_view["crop"]["turn_deg"] if plan_view and plan_view.get("crop") else 0.0
            three_ds.append({
                "index": 1,
                "name": namer.fill("set 3D view name logic", rls_settings.set_template(s, "set_view_name_three_d"),
                                   lead_values, every, {SEQ_COUNTER: 1}),
                "title": title("set_title_on_sheet_three_d", lead_values, 1, set_wide=True),
                "forward": geo.turn_xy(forward, turn),
                "up": geo.turn_xy(up, turn),
                "viewport": three_d_vp,
                # no server walls for a set yet, and the plug-in's own rule judges one room: the walls between the
                # set's rooms must stay, so none are hidden (`rls_write._hide_walls_in_the_way`)
                "server": None,
                "keep_walls": True,
                "sheet": at[0] if at else None,
                "centre_mm": at[1] if at else None,
            })

    schedules = []
    for room_id in set_plan.get("schedules") or []:
        values = values_of.get(room_id) or {}
        at = where.get(("schedule", room_id, None))
        schedules.append({
            "room_id": room_id,
            "room": handle_of.get(room_id),
            "name": namer.fill("schedule view name logic", (s.get("view_name_schedule") or "").strip(),
                               values, [values], {SEQ_COUNTER: 1}, label_of.get(room_id)),
            "title": title("title_on_sheet_schedule", values, 1, label_of.get(room_id)),
            "sheet": at[0] if at else None,
            "centre_mm": at[1] if at else None,
            "size_mm": at[2] if at else None,
        })

    # detail numbers run per sheet, in the order the writer makes the viewports of each kind there
    for i, sheet in enumerate(sheets):
        counts = {
            "elevation": len([e for e in elevations if e["sheet"] == i]),
            "plan": 1 if plan_view and plan_view["sheet"] == i else 0,
            "three_d": len([v for v in three_ds if v["sheet"] == i]),
        }
        sheet["detail_numbers"] = detail_numbers.assign(counts, s)

    plan = {
        "set": True,
        "size": size,
        "title_block": title_block,
        "sheets": sheets,
        "lead": rooms[0][1] if rooms else None,
        "rooms": [handle for _r, handle, _v in rooms],
        "plan_view": plan_view,
        "plan_scale": plan_view["scale"] if plan_view else rls_task.DEFAULT_SCALE,
        "elevations": elevations,
        "three_ds": three_ds,
        "three_d_skipped": skipped,
        "schedules": schedules,
        "plan_view_type": s.get("plan_view_type"),
        "elevation_view_type": s.get("elevation_view_type"),
        "three_d_view_type": s.get("three_d_view_type"),
        "viewport_type": s.get("viewport_type") or None,
        "schedule_base": s.get("schedule_base"),
        "room_tag": s.get("room_tag") or None,
        "elevation_grid": s.get("elevation_grid") or None,
    }
    return plan, aborts


def _names_of(plan):
    names = []
    if plan["plan_view"]:
        names.append(plan["plan_view"]["name"])
    names.extend(e["name"] for e in plan["elevations"])
    names.extend(v["name"] for v in plan["three_ds"])
    names.extend(sc["name"] for sc in plan["schedules"])
    return [n for n in names if n]


def check(task, settings, facts, taken):
    """`(plan, aborts, warnings)` for one set task, as `rls_task.check` answers for one room. `taken` holds the sheet
    numbers and view names the tasks before this one will make."""
    aborts, warnings = [], []
    set_plan = task.get("rls_set") or {}
    lead = task.get("room") or {}
    label = "set {}".format(lead.get("number") or lead.get("name") or task.get("id") or "?")
    missing = rls_settings.missing(settings)
    if missing:
        return None, [finding(SETTINGS_INCOMPLETE, "the RLS settings are incomplete: {}".format(", ".join(missing)))], []

    rooms = []
    for r in set_plan.get("rooms") or [lead]:
        handle = facts.find_room(r.get("number"), r.get("room_id"))
        if handle is None:
            aborts.append(finding(ROOM_MISSING, "{}: room {} is no longer in the model".format(label, r.get("number") or r.get("room_id"))))
            continue
        if facts.room_phase_name(handle) is None:
            aborts.append(finding(PHASE_MISSING, "{}: room {} has no phase".format(label, r.get("number") or r.get("room_id"))))
        rooms.append((r, handle, facts.room_values(handle)))
    if aborts:
        return None, aborts, []

    plan, built = build(task, settings, rooms)
    aborts.extend(built)
    if aborts:
        return plan, aborts, []

    if not facts.title_block_exists(plan["title_block"]):
        aborts.append(finding(NO_TITLE_BLOCK, "{}: the model has no title block '{}' (size {})".format(
            label, plan["title_block"], plan["size"].upper())))
    kinds = [("plan", plan["plan_view_type"], "plan view type")]
    if plan["elevations"]:
        kinds.append(("elevation", plan["elevation_view_type"], "elevation view type"))
    for kind, name, what in kinds:
        if not facts.view_type_exists(kind, name):
            aborts.append(finding(VIEW_TYPE_MISSING, "{}: the {} '{}' is not in the model".format(label, what, name)))
    if plan["viewport_type"] and not facts.view_type_exists("viewport", plan["viewport_type"]):
        aborts.append(finding(VIEW_TYPE_MISSING, "{}: the viewport type '{}' is not in the model".format(label, plan["viewport_type"])))
    if plan["three_ds"]:
        if not plan["three_d_view_type"]:
            aborts.append(finding(VIEW_TYPE_MISSING, "{}: the set has a 3D view and the settings choose no 3D view type".format(label)))
        elif not facts.view_type_exists("three_d", plan["three_d_view_type"]):
            aborts.append(finding(VIEW_TYPE_MISSING, "{}: the 3D view type '{}' is not in the model".format(label, plan["three_d_view_type"])))
    if plan["schedules"]:
        if not facts.schedule_exists(plan["schedule_base"]):
            aborts.append(finding(SCHEDULE_MISSING, "{}: the base schedule '{}' is not in the model".format(label, plan["schedule_base"])))
        elif not facts.schedule_filters_on_room_number(plan["schedule_base"]):
            aborts.append(finding(SCHEDULE_NOT_ROOM_FILTERED, "{}: the base schedule '{}' does not have the room number as its first filter".format(
                label, plan["schedule_base"])))

    numbers = [sh["number"] for sh in plan["sheets"]]
    clash = [n for n in numbers if n in taken["sheets"] or facts.sheet_exists(n)]
    if clash:
        aborts.append(finding(SHEET_EXISTS, "{}: sheet {} already exists".format(label, ", ".join(sorted(set(clash))))))
    if len(set(numbers)) != len(numbers):
        aborts.append(finding(SHEET_EXISTS, "{}: the sheet number logic gives two sheets of the set one number".format(label)))
    names = _names_of(plan)
    in_use = list(facts.views_in_use(names)) + [n for n in names if n.strip().lower() in taken["views"]]
    if in_use:
        aborts.append(finding(VIEW_EXISTS, "{}: a view named {} already exists".format(
            label, ", ".join("'{}'".format(n) for n in sorted(set(in_use))))))
    seen, twice = set(), set()
    for n in names:
        key = n.strip().lower()
        (twice if key in seen else seen).add(key)
    if twice:
        aborts.append(finding(VIEW_EXISTS, "{}: the view name logic gives more than one view of the set the name {}".format(
            label, ", ".join("'{}'".format(n) for n in sorted(set(n for n in names if n.strip().lower() in twice))))))

    if plan["room_tag"] is None or not facts.tag_type_exists("room", plan["room_tag"]):
        warnings.append(finding(NO_ROOM_TAG, "{}: no room tag is defined (or it is not in the model): the rooms are not tagged".format(label)))
    if plan["elevations"] and not plan["plan_view"]:
        warnings.append(finding(NO_PLAN_FOR_ELEVATIONS, "{}: the set's plan is left out, and an elevation marker needs a plan "
                                                        "view: its {} elevation(s) are not made".format(label, len(plan["elevations"]))))
    if plan["three_d_skipped"]:
        warnings.append(finding(THREE_D_NO_DIRECTION, "{}: the template's 3D view has no direction to copy: no 3D view is made".format(label)))
    if plan["elevation_grid"] and plan["elevations"] and not facts.line_detail_exists(plan["elevation_grid"]):
        warnings.append(finding(rls_task.NO_ELEVATION_GRID, "{}: the model has no line based detail family '{}': the elevations get no 2D grid".format(
            label, plan["elevation_grid"])))

    if not aborts:
        taken["sheets"].update(numbers)
        taken["views"].update(n.strip().lower() for n in names)
    return plan, aborts, warnings

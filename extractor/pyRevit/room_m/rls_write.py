"""Make one room layout sheet in Revit, from a checked task. What `room_m.rls_task` planned, done.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

**duHast does the Revit work; this module decides the order and what goes where.** Sheets, views,
viewports, elevation markers, schedules and tags are made by `duHast.Revit.Views`, `.Annotation`,
`.Rooms` and `.Family` (the helpers added for this, and the existing ones it fixed). What stays here is the
arithmetic from the task's millimetres to Revit's feet, and the sequence.

**One task is one transaction group**: when any step fails the group is rolled back and the task is
reported failed, so a failed task leaves no trace (`in_transaction_group`).

**Assumptions to verify in Revit**, each stated where it is made: an elevation marker turned about
its point turns the views on it; a view's `ViewDirection` points at the viewer; the crop box of an
elevation has its far extent at negative z. A view that does not face the way the plan says is
reported as a note, not hidden.

**Not placed yet**: tags in elevation views (the plan has only an offset along the line), the
rectangle drawn round an item a person chose to tag, and a tag's own text. Schedule size is Revit's.

**Not yet run in Revit.**
"""

import math

from Autodesk.Revit.DB import (
    BuiltInCategory,
    BuiltInParameter,
    Element,
    Transaction,
    TransactionGroup,
    ViewFamily,
    XYZ,
)

from duHast.Revit.Annotation.independent_tags_create import create_independent_tag_without_transaction_wrapper
from duHast.Revit.Common.element_id import get_element_id_from_int
from duHast.Revit.Common.transaction import in_transaction, in_transaction_group
from duHast.Revit.Family.family_symbol_lookup import (
    get_family_symbol_by_full_name,
    get_title_block_symbol_by_name,
)
from duHast.Revit.Rooms.room_tags_create import create_room_tag_without_transaction_wrapper
from duHast.Revit.Rooms.rooms import get_room_phase
from duHast.Revit.Views.elevation_marker import (
    create_elevation_marker_without_transaction_wrapper,
    create_elevation_on_marker_without_transaction_wrapper,
    rotate_elevation_marker_without_transaction_wrapper,
    set_elevation_extent_without_transaction_wrapper,
)
from duHast.Revit.Views.plan_view_create import create_view_plan
from duHast.Revit.Views.schedules_create import (
    duplicate_schedule_without_transaction_wrapper,
    place_schedule_on_sheet_without_transaction_wrapper,
    set_first_filter_value_without_transaction_wrapper,
)
from duHast.Revit.Views.sheets import get_sheet_num_to_elem_dict
from duHast.Revit.Views.sheets_create import create_sheet
from duHast.Revit.Views.schedules import get_schedule_by_name
from duHast.Revit.Views.views import get_view_family_type_by_name
from duHast.Revit.Views.view_3d_create import create_view_3d_around_box_without_transaction_wrapper
from duHast.Revit.Views.views_crop import set_view_crop_around_point_without_transaction_wrapper
from duHast.Revit.Views.views_names import get_views_in_name_collision
from duHast.Utilities.Objects.result import Result
from duHast.Revit.Views.viewports_create import create_viewport_without_transaction_wrapper

from room_m import apply_geometry as geo
from room_m import detail_numbers
from room_m import rls_settings
from room_m import task_log
from room_m.apply_facts import RevitFacts

# How far beyond the room a plan view's crop reaches when the template viewport carried none.
CROP_MARGIN_MM = 600.0
# How far a 3D view's section box stands off the room on every side.
SECTION_MARGIN_MM = 600.0
# A view whose direction is this close to what the plan said (dot product) faces it.
FACES_PLAN = 0.9


class RlsFacts(RevitFacts):
    """What the open model can say about an RLS task (the `facts` `room_m.rls_task` asks)."""

    def room_values(self, room):
        """Every parameter of the room by name, as text, for the naming templates."""
        values = {}
        for p in room.Parameters:
            try:
                name = p.Definition.Name
                text = p.AsString()
                if text is None:
                    text = p.AsValueString()
                values.setdefault(name, text if text is not None else "")
            except Exception:
                continue
        return values

    def title_block_exists(self, name):
        return get_title_block_symbol_by_name(self.doc, name) is not None

    def view_type_exists(self, kind, name):
        if kind == "viewport":
            return self.viewport_type(name) is not None
        wanted = {"plan": ViewFamily.FloorPlan, "three_d": ViewFamily.ThreeDimensional}.get(kind, ViewFamily.Elevation)
        return self._view_type(wanted, name) is not None

    def viewport_type(self, name):
        """The viewport type with this name, or None (see `rls_settings.viewport_types`)."""
        for t in rls_settings.viewport_types(self.doc):
            if Element.Name.GetValue(t) == name:
                return t
        return None

    def _view_type(self, family, name):
        return get_view_family_type_by_name(self.doc, family, name)

    def schedule(self, name):
        return get_schedule_by_name(self.doc, name)

    def schedule_exists(self, name):
        return self.schedule(name) is not None

    def schedule_filters_on_room_number(self, name):
        schedule = self.schedule(name)
        try:
            return schedule is not None and rls_settings.first_filter_is_room_number(schedule)
        except Exception:
            return False

    def tag_symbol(self, kind, name):
        category = BuiltInCategory.OST_RoomTags if kind == "room" else BuiltInCategory.OST_MultiCategoryTags
        return get_family_symbol_by_full_name(self.doc, category, name)

    def tag_type_exists(self, kind, name):
        return bool(name) and self.tag_symbol(kind, name) is not None

    def sheet_exists(self, number):
        return number in get_sheet_num_to_elem_dict(self.doc)

    def views_in_use(self, names):
        return get_views_in_name_collision(self.doc, names)


MM_PER_FT = geo.MM_PER_FT


def _ft(mm):
    return geo.mm_to_ft(mm)


class RlsWriter(object):
    def __init__(self, doc, facts):
        self.doc = doc
        self.facts = facts
        self._ports = {}
        self._viewport_type_id = None
        self._level_z = 0.0

    def __call__(self, verdict):
        """Make the sheet of `verdict`; return its notes. Raises, with the model rolled back, when
        a step fails."""
        plan = verdict["plan"]
        notes = []
        self._ports = dict((kind, []) for kind in detail_numbers.KINDS)
        task_log.step("sheet {} {}: transaction group".format(plan["number"], plan["name"]))
        group = TransactionGroup(self.doc, "RoomMate RLS {}".format(plan["number"]))
        outcome = in_transaction_group(group, lambda: self._write(verdict["task"], plan, notes))
        self.facts.refresh()
        if not outcome.status:
            task_log.error("sheet {}: rolled back: {}".format(plan["number"], outcome.message.strip()))
            raise RuntimeError(outcome.message.strip() or "the sheet could not be made")
        task_log.info("sheet {}: made".format(plan["number"]))
        return notes

    # -- the sequence -----------------------------------------------------------------------

    def _write(self, task, plan, notes):
        result = Result()
        try:
            task_log.step("find the room, its level and phase")
            room = plan["room"]
            level = room.Level
            self._level_z = self._level_z_of(room, level)
            # the views belong to the phase the room does, whatever the task says
            phase = get_room_phase(self.doc, room)
            task_log.step("sheet {}".format(plan["number"]))
            vp_type = self.facts.viewport_type(plan["viewport_type"]) if plan.get("viewport_type") else None
            self._viewport_type_id = vp_type.Id if vp_type is not None else None
            sheet = self._sheet(plan)
            task_log.step("plan view")
            plan_view = self._plan_view(plan, room, level, phase, sheet, notes)
            task_log.step("elevations")
            self._elevations(plan, room, level, phase, plan_view, sheet, notes)
            task_log.step("3D views")
            self._three_ds(plan, room, level, phase, sheet, notes)
            task_log.step("schedule")
            self._schedule(plan, room, sheet, notes)
            task_log.step("detail numbers")
            self._number_details(plan)
            task_log.step("tags")
            self._tags(task, plan, room, plan_view, level, notes)
            notes.append("sheet {} made".format(plan["number"]))
        except Exception as e:
            task_log.exception("sheet {} failed".format(plan.get("number")))
            result.update_sep(False, str(e))
        return result

    @staticmethod
    def _level_z_of(room, level):
        """Where the room's level is in the model's own coordinates, feet. **`Level.Elevation` is not that**: on
        a model whose project base point is above the internal origin it is the elevation in the project's frame
        (110250 mm against a floor at 450 mm: measured), while views, crop boxes and geometry are in the internal
        frame. The room's bounding box is: its bottom is the level plus the room's base offset."""
        try:
            base = room.get_Parameter(BuiltInParameter.ROOM_LOWER_OFFSET)
            return room.get_BoundingBox(None).Min.Z - (base.AsDouble() if base is not None else 0.0)
        except Exception:
            return level.Elevation

    def _step(self, name, work):
        """Run `work` (returns a duHast Result) in a transaction of its own; a failure raises."""
        task_log.step(name)
        outcome = in_transaction(Transaction(self.doc, name), work)
        if not outcome.status:
            task_log.error("{}: {}".format(name, outcome.message.strip()))
            raise RuntimeError("{}: {}".format(name, outcome.message.strip()))
        return outcome

    def _regenerate(self):
        """Bring the document up to date. Revit refuses `Regenerate` outside a transaction, so it gets one."""
        def work():
            result = Result()
            self.doc.Regenerate()
            return result

        self._step("RoomMate: regenerate", work)

    def _sheet(self, plan):
        symbol = get_title_block_symbol_by_name(self.doc, plan["title_block"])
        outcome = create_sheet(self.doc, str(plan["number"]), str(plan["name"]), symbol)
        if not outcome.status or not outcome.result:
            raise RuntimeError("sheet {}: {}".format(plan["number"], outcome.message.strip()))
        return outcome.result[0]

    def _plan_view(self, plan, room, level, phase, sheet, notes):
        view_type = self.facts._view_type(ViewFamily.FloorPlan, plan["plan_view_type"])
        outcome = create_view_plan(self.doc, level, view_type, plan["plan_name"], phase)
        if not outcome.status or not outcome.result:
            raise RuntimeError("plan view {}: {}".format(plan["plan_name"], outcome.message.strip()))
        view = outcome.result[0]

        box = room.get_BoundingBox(None)
        room_w = (box.Max.X - box.Min.X) * geo.MM_PER_FT + 2 * CROP_MARGIN_MM
        room_h = (box.Max.Y - box.Min.Y) * geo.MM_PER_FT + 2 * CROP_MARGIN_MM
        vp = plan["plan_viewport"]
        size = (geo.crop_size_mm(vp, room_w, room_h) if vp else None) or (room_w, room_h)
        centre = XYZ((box.Max.X + box.Min.X) / 2.0, (box.Max.Y + box.Min.Y) / 2.0, self._level_z)

        def crop():
            view.Scale = int(plan["plan_scale"])
            return set_view_crop_around_point_without_transaction_wrapper(view, centre, _ft(size[0]), _ft(size[1]))

        self._step("RoomMate: crop {}".format(plan["plan_name"]), crop)
        self._viewport(sheet, view, vp, notes, "plan", "plan")
        return view

    def _viewport(self, sheet, view, template_vp, notes, label, kind):
        at = (geo.viewport_centre_ft(template_vp) if template_vp else None)
        if at is None:
            at = (0.0, 0.0)
            notes.append("the {} viewport had no place on the template sheet: put at the sheet's origin".format(label))
        self._regenerate()
        type_id = self._viewport_type_id
        label_offset, label_length = geo.viewport_label_ft(template_vp)
        outcome = self._step(
            "RoomMate: viewport {}".format(view.Name),
            lambda: create_viewport_without_transaction_wrapper(
                self.doc, sheet, view, XYZ(at[0], at[1], 0.0), type_id,
                XYZ(label_offset[0], label_offset[1], 0.0) if label_offset else None, label_length),
        )
        port = outcome.result[0] if outcome.result else None
        if port is not None:
            self._ports[kind].append(port)
        return port

    def _number_details(self, plan):
        """Give the viewports their detail numbers (`room_m.detail_numbers`). Revit numbers a new viewport
        itself and a number may be used once per sheet, so each viewport is first parked on a place holder
        and only then given its real number: an automatic number can never block one of ours."""
        ports = [(kind, port) for kind in detail_numbers.KINDS for port in self._ports.get(kind, [])]
        wanted = []
        for kind in detail_numbers.KINDS:
            labels = plan["detail_numbers"].get(kind, [])
            # a viewport that failed to be made leaves its label unused rather than shifting the rest
            wanted.extend(labels[:len(self._ports.get(kind, []))])
        if not ports:
            return

        def put(values):
            result = Result()
            try:
                for (_kind, port), value in zip(ports, values):
                    port.get_Parameter(BuiltInParameter.VIEWPORT_DETAIL_NUMBER).Set(value)
            except Exception as e:
                result.update_sep(False, str(e))
            return result

        self._step("RoomMate: detail numbers (place holders)", lambda: put(detail_numbers.placeholders(len(ports))))
        self._step("RoomMate: detail numbers", lambda: put(wanted))

    def _elevations(self, plan, room, level, phase, plan_view, sheet, notes):
        if not plan["elevations"]:
            return
        view_type = self.facts._view_type(ViewFamily.Elevation, plan["elevation_view_type"])
        # the bottom of every elevation is the floor of THIS room: its level plus its base offset
        base = room.get_Parameter(BuiltInParameter.ROOM_LOWER_OFFSET)
        bottom = self._level_z + (base.AsDouble() if base is not None else 0.0)
        by_marker = {}
        order = []
        for e in plan["elevations"]:
            key = e["marker_id"]
            if key not in by_marker:
                by_marker[key] = []
                order.append(key)
            by_marker[key].append(e)
        for key in order:
            self._marker(by_marker[key], view_type, level, phase, plan_view, plan, sheet, notes, bottom)

    def _marker(self, elevations, view_type, level, phase, plan_view, plan, sheet, notes, bottom):
        first = elevations[0]
        loc = first["marker_location"]
        location = XYZ(_ft(loc[0]), _ft(loc[1]), self._level_z)
        scale = first["scale"] or plan["plan_scale"]
        marker = self._step(
            "RoomMate: elevation marker",
            lambda: create_elevation_marker_without_transaction_wrapper(self.doc, view_type, location, scale),
        ).result[0]

        made = []
        for e in elevations:
            outcome = self._step(
                "RoomMate: {}".format(e["name"]),
                lambda e=e: create_elevation_on_marker_without_transaction_wrapper(
                    self.doc, marker, plan_view, e["marker_index"], e["name"], phase.Id),
            )
            made.append((e, outcome.result[0]))

        # Which way index 0 of a new marker looks is Revit's to say: turn the marker by how far the
        # first view is from where the plan wants it, then check every view.
        first_view = made[0][1]
        wanted = math.atan2(first["looks"][1], first["looks"][0])
        direction = first_view.ViewDirection
        actual = math.atan2(-direction.Y, -direction.X)
        delta = math.atan2(math.sin(wanted - actual), math.cos(wanted - actual))
        if abs(delta) > 1e-3:
            self._step(
                "RoomMate: turn elevation marker",
                lambda: rotate_elevation_marker_without_transaction_wrapper(self.doc, marker, location, delta),
            )
        self._regenerate()

        for e, view in made:
            line_start = XYZ(_ft(e["line_start"][0]), _ft(e["line_start"][1]), self._level_z)
            line_end = XYZ(_ft(e["line_end"][0]), _ft(e["line_end"][1]), self._level_z)
            self._step(
                "RoomMate: extent of {}".format(e["name"]),
                lambda view=view, e=e, s=line_start, t=line_end: set_elevation_extent_without_transaction_wrapper(
                    view, s, t, _ft(e["depth_mm"]) if e.get("depth_mm") else None,
                    _ft(e["height_mm"]) if e.get("height_mm") else None, bottom),
            )
            self._check_vertical_extent(view, e, bottom, notes)
            d = view.ViewDirection
            looks = e["looks"]
            if (-d.X * looks[0] - d.Y * looks[1]) < FACES_PLAN:
                notes.append("{} does not face the way the plan says (the marker's other views are at fixed quarter turns)".format(e["name"]))
            self._viewport(sheet, view, e["viewport"], notes, e["name"], "elevation")

    def _check_vertical_extent(self, view, e, bottom, notes):
        """Say so when the crop box is not where it was asked to be: its bottom the room's floor, its top the
        template's height above that."""
        try:
            crop = view.CropBox
            low = crop.Min.Y + crop.Transform.Origin.Z
            high = crop.Max.Y + crop.Transform.Origin.Z
        except Exception:
            return
        if abs(low - bottom) > _ft(2.0):
            notes.append("{}: the crop box starts {:.0f} mm from the room's floor".format(e["name"], abs(low - bottom) * MM_PER_FT))
        if e.get("height_mm") and abs((high - low) - _ft(e["height_mm"])) > _ft(2.0):
            notes.append("{}: the crop box is {:.0f} mm tall, the template's is {:.0f} mm".format(
                e["name"], (high - low) * MM_PER_FT, e["height_mm"]))

    def _three_ds(self, plan, room, level, phase, sheet, notes):
        """A new 3D view for each 3D viewport of the template: the template's direction, looking at THIS
        room, cut to the room plus a margin. The template view itself is never placed."""
        if not plan["three_ds"]:
            return
        view_type = self.facts._view_type(ViewFamily.ThreeDimensional, plan["three_d_view_type"])
        if view_type is None:
            notes.append("the model has no 3D view type '{}': no 3D view was made".format(plan["three_d_view_type"]))
            return
        box = room.get_BoundingBox(None)
        margin = _ft(SECTION_MARGIN_MM)
        low = XYZ(box.Min.X - margin, box.Min.Y - margin, min(box.Min.Z, self._level_z) - margin)
        high = XYZ(box.Max.X + margin, box.Max.Y + margin, box.Max.Z + margin)
        for v in plan["three_ds"]:
            view = self._step(
                "RoomMate: {}".format(v["name"]),
                lambda v=v: create_view_3d_around_box_without_transaction_wrapper(
                    self.doc, view_type, v["name"], v["forward"], v["up"], low, high, phase),
            ).result[0]
            self._viewport(sheet, view, v["viewport"], notes, v["name"], "three_d")

    def _schedule(self, plan, room, sheet, notes):
        base = self.facts.schedule(plan["schedule_base"])
        copy = self._step(
            "RoomMate: schedule {}".format(plan["schedule_name"]),
            lambda: duplicate_schedule_without_transaction_wrapper(self.doc, base, plan["schedule_name"]),
        ).result[0]
        self._step(
            "RoomMate: schedule filter",
            lambda: set_first_filter_value_without_transaction_wrapper(copy, str(room.Number)),
        )
        vp = plan["schedule_viewport"]
        box = (vp or {}).get("bounding_box") or {}
        try:
            # the schedule's upper left corner: the template box's smallest x and largest y (mm)
            at = (_ft(box["min_x"]), _ft(box["max_y"]))
        except (KeyError, TypeError):
            at = (0.0, 0.0)
            notes.append("the schedule had no place on the template sheet: put at the sheet's origin")
        self._step(
            "RoomMate: place schedule",
            lambda: place_schedule_on_sheet_without_transaction_wrapper(self.doc, sheet, copy, XYZ(at[0], at[1], 0.0)),
        )

    def _tags(self, task, plan, room, plan_view, level, notes):
        z = self._level_z
        independent = self.facts.tag_symbol("independent", plan["independent_tag"]) if plan["independent_tag"] else None
        room_tag = self.facts.tag_symbol("room", plan["room_tag"]) if plan["room_tag"] else None
        made, problems = 0, []
        if independent is not None:
            for tag in plan["tags"]:
                try:
                    element = self.doc.GetElement(get_element_id_from_int(int(tag["item_id"])))
                    head = XYZ(_ft(tag["head"][0]), _ft(tag["head"][1]), z)
                    outcome = self._step(
                        "RoomMate: tag",
                        lambda element=element, head=head: create_independent_tag_without_transaction_wrapper(
                            self.doc, plan_view, element, head, independent.Id),
                    )
                    made += 1 if outcome.status else 0
                except Exception as e:
                    problems.append("tag on item {} not placed: {}".format(tag.get("item_id"), e))
        if room_tag is not None:
            head = None
            if plan.get("room_tag_head"):
                # The server worked the place out against this room's boundary (plan decision 27).
                head = XYZ(_ft(plan["room_tag_head"][0]), _ft(plan["room_tag_head"][1]), z)
            else:
                # A rule learned before its room was on the server: the offset in the plan's axes.
                for rule in geo.room_rules(task.get("tag_rules") or [], geo.PLAN):
                    point = room.Location.Point
                    placement = geo.tag_placement(
                        geo.room_like(point.X * geo.MM_PER_FT, point.Y * geo.MM_PER_FT), rule, plan["plan_scale"])
                    if placement:
                        head = XYZ(_ft(placement["head"][0]), _ft(placement["head"][1]), z)
                    break
            try:
                self._step(
                    "RoomMate: room tag",
                    lambda: create_room_tag_without_transaction_wrapper(self.doc, plan_view, room, room_tag.Id, head),
                )
            except Exception as e:
                problems.append("room tag not placed: {}".format(e))
        notes.append("{} tag(s) placed".format(made))
        for p in problems[:5]:
            notes.append(p)
        if len(problems) > 5:
            notes.append("... and {} more tags not placed".format(len(problems) - 5))

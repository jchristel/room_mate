"""Make the sheets of a SET of rooms (multi-room RLS) in Revit, from a checked set task. What `room_m.rls_set_plan`
planned, done.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

**The single-room writer's steps, with the set's own places.** `RlsSetWriter` is an `RlsWriter`: the plan view's crop
and turn, each elevation marker and its views, the 3D view, the Title on Sheet, the annotation crop, the detail numbers
and the 2D grids are its code, unchanged. What differs is WHERE each viewport goes: the task says its sheet and its
centre (mm from the paper's bottom left, y up), which `_place_on` answers, so nothing reads the template's places but
the label offset and the annotation crop. **The paper's bottom left is taken to be the sheet's origin**, which is where
a title block family is usually drawn from; one drawn about another point puts every viewport off by that much.

**The whole set is one transaction group**: a failure anywhere rolls back every sheet of the set, so a set is made
whole or not at all, as a single sheet is.

A set's elevations each sit in their own room (marker at its box's centre, floor at its own level and base offset,
crop as tall as its clear height); the plan and the 3D view are on the lead room's level and in its phase. The rooms
are tagged in the plan where Revit puts a room tag by default (no tag rules for a set yet). Item tags are not placed.

**Not yet run in Revit.**
"""

from Autodesk.Revit.DB import BuiltInParameter, TransactionGroup, ViewFamily, XYZ

from duHast.Revit.Common.transaction import in_transaction_group
from duHast.Revit.Rooms.room_tags_create import create_room_tag_without_transaction_wrapper
from duHast.Revit.Rooms.rooms import get_room_phase
from duHast.Revit.Views.schedules_create import (
    duplicate_schedule_without_transaction_wrapper,
    place_schedule_on_sheet_without_transaction_wrapper,
    set_first_filter_value_without_transaction_wrapper,
)
from duHast.Utilities.Objects.result import Result

from room_m import detail_numbers
from room_m import task_log
from room_m.rls_write import RlsWriter, _ft
from room_m.task_plan import info
from room_m.utils.rooms import level_z_ft


class RlsSetWriter(RlsWriter):
    def __init__(self, doc, facts):
        RlsWriter.__init__(self, doc, facts)
        self._sheets = []
        self._ports_by_sheet = {}

    def __call__(self, verdict):
        """Make every sheet of the set in `verdict`; return the notes. Raises, with the model rolled back, when a step
        fails."""
        plan = verdict["plan"]
        notes = []
        self._elevation_views = {}
        self._ports_by_sheet = {}
        first = plan["sheets"][0]["number"] if plan["sheets"] else "?"
        task_log.step("set {} ({} sheet(s)): transaction group".format(first, len(plan["sheets"])))
        group = TransactionGroup(self.doc, "RoomMate RLS set {}".format(first))
        outcome = in_transaction_group(group, lambda: self._write_set(plan, notes))
        self.facts.refresh()
        if not outcome.status:
            task_log.error("set {}: rolled back: {}".format(first, outcome.message.strip()))
            raise RuntimeError(outcome.message.strip() or "the set could not be made")
        task_log.info("set {}: made".format(first))
        return notes

    # -- where a viewport goes -------------------------------------------------------------------

    def _place_on(self, item, sheet):
        """The item's own sheet and centre, from the task. An item the task did not place goes on the first sheet at
        the template's place, which the planner never plans (the server refuses such a layout)."""
        index = item.get("sheet")
        if index is None or not (0 <= index < len(self._sheets)):
            return (self._sheets[0] if self._sheets else sheet), None
        return self._sheets[index], item.get("centre_mm")

    def _viewport(self, sheet, view, template_vp, notes, label, kind, at_mm=None):
        port = RlsWriter._viewport(self, sheet, view, template_vp, notes, label, kind, at_mm)
        if port is not None:
            # the base keeps one list per kind for one sheet; a set numbers each sheet's own
            self._ports[kind].pop()
            self._ports_by_sheet.setdefault(self._sheet_index(sheet), dict((k, []) for k in detail_numbers.KINDS))[kind].append(port)
        return port

    def _sheet_index(self, sheet):
        for i, s in enumerate(self._sheets):
            if s.Id == sheet.Id:
                return i
        return 0

    # -- the sequence ----------------------------------------------------------------------------

    def _write_set(self, plan, notes):
        result = Result()
        try:
            self._ports = dict((kind, []) for kind in detail_numbers.KINDS)
            lead = plan["lead"]
            level = lead.Level
            self._level_z = level_z_ft(lead, level)
            phase = get_room_phase(self.doc, lead)
            vp_type = self.facts.viewport_type(plan["viewport_type"]) if plan.get("viewport_type") else None
            self._viewport_type_id = vp_type.Id if vp_type is not None else None

            self._sheets = []
            for sheet in plan["sheets"]:
                task_log.step("sheet {}".format(sheet["number"]))
                self._sheets.append(self._sheet(dict(sheet, title_block=plan["title_block"])))

            plan_view = None
            if plan.get("plan_view"):
                task_log.step("plan view")
                plan_view = self._plan_view(self._single(plan), lead, level, phase, self._sheets[0], notes)
            task_log.step("elevations")
            self._set_elevations(plan, plan_view, notes)
            if plan["three_ds"]:
                task_log.step("3D view")
                self._three_ds(self._single(plan), lead, level, phase, self._sheets[0], notes)
            task_log.step("schedules")
            for schedule in plan["schedules"]:
                self._set_schedule(schedule, plan["schedule_base"], notes)
            task_log.step("detail numbers")
            for i, sheet in enumerate(plan["sheets"]):
                self._ports = self._ports_by_sheet.get(i) or dict((k, []) for k in detail_numbers.KINDS)
                self._number_details(sheet)
            if plan_view is not None:
                task_log.step("room tags")
                self._room_tags(plan, plan_view, notes)
            task_log.step("2D grids")
            self._elevation_grids(plan, notes)
            notes.append(info("sheet(s) {} made".format(", ".join(s["number"] for s in plan["sheets"]))))
        except Exception as e:
            task_log.exception("set {} failed".format(plan["sheets"][0]["number"] if plan.get("sheets") else "?"))
            result.update_sep(False, str(e))
        return result

    def _single(self, plan):
        """The set's plan view in the keys the single-room steps read (`_plan_view`, `_three_ds`)."""
        view = plan.get("plan_view") or {}
        return {
            "plan_view_type": plan["plan_view_type"],
            "plan_name": view.get("name"),
            "plan_title": view.get("title"),
            "plan_viewport": view.get("viewport"),
            "plan_scale": view.get("scale"),
            "plan_crop": view.get("crop"),
            "sheet": view.get("sheet"),
            "centre_mm": view.get("centre_mm"),
            "three_ds": plan["three_ds"],
            "three_d_view_type": plan["three_d_view_type"],
        }

    def _set_elevations(self, plan, plan_view, notes):
        """Each room's elevations on its own marker: the room's own level and floor, and a plan view to host the
        marker (the set's plan; without one, nothing can host a marker and the elevations are a note)."""
        if not plan["elevations"]:
            return
        if plan_view is None:
            notes.append("no elevation made: the set's plan is left out, and an elevation marker needs a plan view")
            return
        view_type = self.facts._view_type(ViewFamily.Elevation, plan["elevation_view_type"])
        by_room, order = {}, []
        for e in plan["elevations"]:
            if e["room_id"] not in by_room:
                by_room[e["room_id"]] = []
                order.append(e["room_id"])
            by_room[e["room_id"]].append(e)
        lead_z = self._level_z
        for room_id in order:
            views = by_room[room_id]
            room = views[0]["room"]
            level = room.Level
            self._level_z = level_z_ft(room, level)
            base = room.get_Parameter(BuiltInParameter.ROOM_LOWER_OFFSET)
            bottom = self._level_z + (base.AsDouble() if base is not None else 0.0)
            phase = get_room_phase(self.doc, room)
            self._marker(views, view_type, level, phase, plan_view, plan, self._sheets[0], notes, bottom)
        self._level_z = lead_z

    def _set_schedule(self, schedule, base_name, notes):
        """A room's schedule: the base duplicated, its first filter the room's number, and placed WHOLE on each sheet the
        layout put it on, its upper left corner where the task's centre and estimated size say (Revit decides its real
        size). Never split (decided 2026-11-14)."""
        base = self.facts.schedule(base_name)
        room = schedule["room"]
        copy = self._step(
            "RoomMate: schedule {}".format(schedule["name"]),
            lambda: duplicate_schedule_without_transaction_wrapper(self.doc, base, schedule["name"]),
        ).result[0]
        self._step(
            "RoomMate: schedule filter {}".format(schedule["name"]),
            lambda: set_first_filter_value_without_transaction_wrapper(copy, str(room.Number)),
        )
        self._title_on_sheet(copy, schedule.get("title"), schedule["name"], notes)
        # the same schedule, whole, on every sheet the layout put it on (a schedule may be placed more than once)
        for place in schedule.get("places") or [schedule]:
            sheet, centre = self._place_on(place, None)
            size = place.get("size_mm") or [0.0, 0.0]
            if centre:
                at = (_ft(centre[0] - size[0] / 2.0), _ft(centre[1] + size[1] / 2.0))
            else:
                at = (0.0, 0.0)
                notes.append("{}: no place on the set's sheets: put at the sheet's origin".format(schedule["name"]))
            self._step(
                "RoomMate: place schedule {}".format(schedule["name"]),
                lambda sheet=sheet, at=at: place_schedule_on_sheet_without_transaction_wrapper(
                    self.doc, sheet, copy, XYZ(at[0], at[1], 0.0)),
            )

    def _room_tags(self, plan, plan_view, notes):
        """Tag every room of the set in the plan where Revit puts a room tag (no rule: the head is left to Revit). A
        failure is a note."""
        symbol = self.facts.tag_symbol("room", plan["room_tag"]) if plan.get("room_tag") else None
        if symbol is None:
            return
        made = 0
        for room in plan["rooms"]:
            try:
                self._step(
                    "RoomMate: room tag {}".format(room.Number),
                    lambda room=room: create_room_tag_without_transaction_wrapper(self.doc, plan_view, room, symbol.Id, None),
                )
                made += 1
            except Exception as e:
                notes.append("room {}: room tag not placed ({})".format(room.Number, e))
        notes.append(info("{} room tag(s) placed".format(made)))

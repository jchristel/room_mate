"""Write one room task into the open Revit model.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

The decisions are not here: `room_m.task_plan` has already checked the task, and
`room_m.apply_geometry` works out where things go. This module turns those answers into Revit
calls, one task per `TransactionGroup`, so a task that fails leaves the model as it found it.

**What it does, and does not.**

- Items: places each task item as a new instance, in the task's phase, on the room's level. A
  family placed on a level is rotated to the item's facing; one hosted on a wall is put on the
  nearest wall within `WALL_REACH_MM` and flipped to face the right way. Anything else (face
  or work plane based) is reported and left out. The task's instance and type parameters are
  NOT applied yet, and the height is the level's.
- Sheets are not made here: a `create_rls` task goes through the RLS create button (`room_m.rls_task`,
  `room_m.rls_write`); a populate task that asks for sheets is warned about and places items only.

**Not yet run in Revit.**
"""

from Autodesk.Revit.DB import (
    BuiltInParameter,
    ElementTransformUtils,
    FamilyPlacementType,
    FilteredElementCollector,
    Line,
    Transaction,
    TransactionGroup,
    TransactionStatus,
    Wall,
    XYZ,
)
from Autodesk.Revit.DB.Structure import StructuralType

from room_m import apply_geometry as geo
from room_m import task_plan
from room_m.utils.generic import element_id_str, phase_by_name

# How far from a wall's centre line a hosted item may sit and still be put on that wall.
WALL_REACH_MM = 450.0
MAX_NOTES_PER_KIND = 5


class TaskWriter(object):
    def __init__(self, doc, facts):
        self.doc = doc
        self.facts = facts

    # -- the one entry point ----------------------------------------------------------------

    def __call__(self, verdict):
        """Write the task of `verdict`; return its notes. Raises, with the model rolled back,
        when it cannot."""
        task = verdict["task"]
        group = TransactionGroup(self.doc, "RoomMate task {}".format((task.get("room") or {}).get("number", "")))
        group.Start()
        try:
            notes = self._write(task)
            group.Assimilate()
        except Exception:
            group.RollBack()
            self.facts.refresh()
            raise
        self.facts.refresh()
        return notes

    def _write(self, task):
        room_ref = task.get("room") or {}
        room = self.facts.find_room(room_ref.get("number"), room_ref.get("room_id"))
        if room is None:
            raise ValueError("the room is no longer in the model")
        level = room.Level
        phase = phase_by_name(self.doc, task["phase"])
        notes = []

        if task.get("kind") == task_plan.POPULATE:
            if task.get("overwrite_layout"):
                self._clear_room(room, task["phase"], notes)
            self._place_items(task, level, phase, notes)
        return notes

    # -- helpers ----------------------------------------------------------------------------

    def _in_transaction(self, name, work):
        t = Transaction(self.doc, name)
        t.Start()
        try:
            result = work()
            if t.Commit() != TransactionStatus.Committed:
                raise RuntimeError("Revit did not commit '{}'".format(name))
            return result
        except Exception:
            if t.HasStarted() and not t.HasEnded():
                t.RollBack()
            raise

    @staticmethod
    def _capped(notes, kind, messages):
        """Add `messages` to `notes`, at most `MAX_NOTES_PER_KIND`, then a count of the rest."""
        for m in messages[:MAX_NOTES_PER_KIND]:
            notes.append(m)
        if len(messages) > MAX_NOTES_PER_KIND:
            notes.append("... and {} more {}".format(len(messages) - MAX_NOTES_PER_KIND, kind))

    # -- items ------------------------------------------------------------------------------

    def _clear_room(self, room, phase_name, notes):
        existing = self.facts.room_items(room, phase_name)

        def work():
            gone = 0
            for instance in existing:
                try:
                    self.doc.Delete(instance.Id)
                    gone += 1
                except Exception:
                    pass
            return gone

        gone = self._in_transaction("RoomMate: clear room", work)
        # What was read about the room's content now names deleted elements.
        self.facts.refresh()
        notes.append("removed {} existing item(s) from the room before placing".format(gone))

    def _place_items(self, task, level, phase, notes):
        items = task.get("items") or []
        placed = []
        problems = []

        def work():
            for item in items:
                try:
                    instance = self._place_item(item, level, phase)
                    if instance is not None:
                        placed.append(instance)
                except Exception as e:
                    problems.append("item '{}' / '{}' not placed: {}".format(item.get("family_name"), item.get("type_name"), e))

        self._in_transaction("RoomMate: place items", work)
        self._capped(notes, "items not placed", problems)
        notes.append("placed {} of {} item(s)".format(len(placed), len(items)))
        self.doc.Regenerate()
        # The room's content now includes what was placed; the next read must see it.
        self.facts.refresh()
        return placed

    def _symbol(self, item):
        family = item.get("family_name")
        type_name = item.get("type_name")
        symbol = self.facts.symbol_for(family, type_name)
        if symbol is None:
            path = self.facts.library_path(family)
            if path is None:
                raise ValueError("family is not in the model or the library")
            loaded = self.doc.LoadFamily(path)
            if not (loaded[0] if isinstance(loaded, tuple) else loaded):
                raise ValueError("the library family could not be loaded")
            self.facts.forget_symbols()
            symbol = self.facts.symbol_for(family, type_name)
            if symbol is None:
                raise ValueError("the loaded family has no type '{}'".format(type_name))
        if not symbol.IsActive:
            symbol.Activate()
            self.doc.Regenerate()
        return symbol

    def _place_item(self, item, level, phase):
        symbol = self._symbol(item)
        point = XYZ(geo.mm_to_ft(item["x_mm"]), geo.mm_to_ft(item["y_mm"]), level.Elevation)
        kind = symbol.Family.FamilyPlacementType
        if kind == FamilyPlacementType.OneLevelBased:
            instance = self.doc.Create.NewFamilyInstance(point, symbol, level, StructuralType.NonStructural)
            angle = geo.facing_angle(item)
            if angle:
                axis = Line.CreateBound(point, XYZ(point.X, point.Y, point.Z + 1.0))
                ElementTransformUtils.RotateElement(self.doc, instance.Id, axis, angle)
        elif kind == FamilyPlacementType.OneLevelBasedHosted:
            wall = self._nearest_wall(level, point)
            if wall is None:
                raise ValueError("no wall within {:.0f} mm to host it".format(WALL_REACH_MM))
            instance = self.doc.Create.NewFamilyInstance(point, symbol, wall, level, StructuralType.NonStructural)
            self._face(instance, item)
        else:
            raise ValueError("placement type {} is not supported".format(kind))
        created = instance.get_Parameter(BuiltInParameter.PHASE_CREATED)
        if created is not None and not created.IsReadOnly:
            created.Set(phase.Id)
        return instance

    def _nearest_wall(self, level, point):
        best = None
        best_distance = geo.mm_to_ft(WALL_REACH_MM)
        for wall in FilteredElementCollector(self.doc).OfClass(Wall):
            try:
                if element_id_str(wall.LevelId) != element_id_str(level.Id):
                    continue
                curve = wall.Location.Curve
                flat = XYZ(point.X, point.Y, curve.GetEndPoint(0).Z)
                distance = curve.Project(flat).Distance
            except Exception:
                continue
            if distance < best_distance:
                best, best_distance = wall, distance
        return best

    def _face(self, instance, item):
        """Flip a hosted instance whose facing points away from the item's."""
        try:
            x = instance.GetTransform().BasisX
            dot = x.X * (item.get("facing_x") or 0.0) + x.Y * (item.get("facing_y") or 0.0)
            if dot < 0 and instance.CanFlipFacing:
                instance.flipFacing()
        except Exception:
            pass

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
- Sheets: a sheet per task sheet, in a title block chosen by size, with a floor plan view for
  each floor plan viewport (a fresh view of the room at the exported scale and crop, placed
  where the viewport was). Elevation, 3D and schedule viewports are reported, not made.
- Tags: for each floor plan view, each element in the room gets the tags its names have a rule
  for, at the rule's place for that element (`apply_geometry.tag_placement`), and the room gets
  its room tag where the room tag rule puts it (offset from the room's point, in the plan's axes).
  Space and area tags are not made. A tag's own rotation is not applied (Revit's tag orientation
  is).

**Not yet run in Revit.**
"""

from Autodesk.Revit.DB import (
    BoundingBoxXYZ,
    BuiltInParameter,
    ElementTransformUtils,
    FamilyPlacementType,
    FilteredElementCollector,
    IndependentTag,
    LinkElementId,
    LeaderEndCondition,
    Line,
    Reference,
    TagMode,
    TagOrientation,
    Transaction,
    TransactionGroup,
    TransactionStatus,
    UV,
    ViewFamily,
    ViewFamilyType,
    ViewPlan,
    ViewSheet,
    Viewport,
    Wall,
    XYZ,
)
from Autodesk.Revit.DB.Structure import StructuralType

from room_m import apply_geometry as geo
from room_m import task_plan
from room_m.apply_facts import name_of
from room_m.utils.generic import element_id_str, phase_by_name

# How far from a wall's centre line a hosted item may sit and still be put on that wall.
WALL_REACH_MM = 450.0
# Room margin added to a view's crop when the export carried none.
CROP_MARGIN_MM = 600.0
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
            notes = self._write(task, verdict)
            group.Assimilate()
        except Exception:
            group.RollBack()
            self.facts.refresh()
            raise
        self.facts.refresh()
        return notes

    def _write(self, task, verdict):
        room_ref = task.get("room") or {}
        room = self.facts.find_room(room_ref.get("number"), room_ref.get("room_id"))
        if room is None:
            raise ValueError("the room is no longer in the model")
        level = room.Level
        phase = phase_by_name(self.doc, task["phase"])
        notes = []
        placed = []

        if task.get("kind") == task_plan.POPULATE:
            if task.get("overwrite_layout"):
                self._clear_room(room, task["phase"], notes)
            placed = self._place_items(task, level, phase, notes)

        if task_plan._wants_rls(task) and not verdict["skip_sheets"]:
            self._make_sheets(task, room, level, phase, placed, notes)
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

    # -- sheets -----------------------------------------------------------------------------

    def _room_label(self, room):
        try:
            name = room.get_Parameter(BuiltInParameter.ROOM_NAME).AsString() or ""
        except Exception:
            name = ""
        return "{} {}".format(room.Number, name).strip()

    def _plan_view_type(self):
        for t in FilteredElementCollector(self.doc).OfClass(ViewFamilyType):
            if t.ViewFamily == ViewFamily.FloorPlan:
                return t
        return None

    def _unused_view_name(self, base):
        taken = set(name_of(v) for v in FilteredElementCollector(self.doc).OfClass(ViewPlan))
        name, n = base, 1
        while name in taken:
            n += 1
            name = "{} ({})".format(base, n)
        return name

    def _make_sheets(self, task, room, level, phase, placed, notes):
        sheets = task.get("sheets") or []
        numbers = self.facts.sheet_numbers_for(task, len(sheets))
        view_type = self._plan_view_type()
        targets = None
        for index, (sheet, number) in enumerate(zip(sheets, numbers)):
            title_block = self.facts.titleblock_for(sheet)
            existing = self.facts.sheet_with_number(number)
            if existing is not None:
                if not task.get("overwrite_rls"):
                    notes.append("sheet {} already exists: not made".format(number))
                    continue
                self._in_transaction("RoomMate: remove old sheet", lambda: self.doc.Delete(existing.Id))
                notes.append("sheet {} replaced (its views are kept)".format(number))
            if title_block is None:
                notes.append("sheet {}: no title block, not made".format(number))
                continue
            if targets is None:
                targets = self._tag_targets(task, room, placed)
            name = "{} {}".format(self._room_label(room), sheet.get("sheet_name") or "Room layout").strip()
            self._make_sheet(task, sheet, number, name, title_block, room, level, phase, view_type, targets, index, notes)

    def _make_sheet(self, task, sheet, number, name, title_block, room, level, phase, view_type, targets, index, notes):
        view_sheet = self._in_transaction("RoomMate: sheet {}".format(number), lambda: self._new_sheet(title_block, number, name))
        made = 0
        for vp in geo.viewports_of(sheet):
            vp_type = (vp.get("vp_type") or "").lower()
            if vp_type != geo.PLAN:
                notes.append("sheet {}: a {} viewport was not made (only floor plans are)".format(number, vp_type or "?"))
                continue
            if view_type is None:
                notes.append("sheet {}: the model has no floor plan view type".format(number))
                continue
            self._plan_viewport(task, view_sheet, vp, room, level, phase, view_type, targets, number, index, made, notes)
            made += 1
        notes.append("sheet {} made with {} viewport(s)".format(number, made))

    def _new_sheet(self, title_block, number, name):
        view_sheet = ViewSheet.Create(self.doc, title_block.Id)
        view_sheet.SheetNumber = number
        view_sheet.Name = name
        return view_sheet

    def _plan_viewport(self, task, view_sheet, vp, room, level, phase, view_type, targets, number, sheet_index, vp_index, notes):
        scale = geo.viewport_scale(vp)
        room_box = room.get_BoundingBox(None)
        room_w = (room_box.Max.X - room_box.Min.X) * geo.MM_PER_FT + 2 * CROP_MARGIN_MM
        room_h = (room_box.Max.Y - room_box.Min.Y) * geo.MM_PER_FT + 2 * CROP_MARGIN_MM
        size = geo.crop_size_mm(vp, room_w, room_h) or (room_w, room_h)
        centre = XYZ((room_box.Max.X + room_box.Min.X) / 2.0, (room_box.Max.Y + room_box.Min.Y) / 2.0, level.Elevation)
        sheet_at = geo.viewport_centre_ft(vp) or (0.0, 0.0)

        def work():
            view = ViewPlan.Create(self.doc, view_type.Id, level.Id)
            view.Name = self._unused_view_name("{} RLS {}.{}".format(self._room_label(room), sheet_index + 1, vp_index + 1))
            if scale:
                view.Scale = int(scale)
            view_phase = view.get_Parameter(BuiltInParameter.VIEW_PHASE)
            if view_phase is not None and not view_phase.IsReadOnly:
                view_phase.Set(phase.Id)
            box = view.CropBox
            local = box.Transform.Inverse.OfPoint(centre)
            half_w = geo.mm_to_ft(size[0]) / 2.0
            half_h = geo.mm_to_ft(size[1]) / 2.0
            new_box = BoundingBoxXYZ()
            new_box.Transform = box.Transform
            new_box.Min = XYZ(local.X - half_w, local.Y - half_h, box.Min.Z)
            new_box.Max = XYZ(local.X + half_w, local.Y + half_h, box.Max.Z)
            view.CropBoxActive = True
            view.CropBoxVisible = False
            view.CropBox = new_box
            Viewport.Create(self.doc, view_sheet.Id, view.Id, XYZ(sheet_at[0], sheet_at[1], 0.0))
            return view

        view = self._in_transaction("RoomMate: view for sheet {}".format(number), work)
        self.doc.Regenerate()
        self._tag_view(task, view, scale, targets, level, number, notes, room)

    # -- tags -------------------------------------------------------------------------------

    def _tag_targets(self, task, room, placed):
        """`[(instance, item-like)]` for everything in the room a tag may go on: the items this
        task placed and the ones already there. Read back from the instances, so a tag is
        placed from what is in the model and not from what the task asked for."""
        instances = list(placed)
        seen = set(element_id_str(i.Id) for i in instances)
        for instance in self.facts.room_items(room, task["phase"]):
            if element_id_str(instance.Id) not in seen:
                instances.append(instance)
        out = []
        for instance in instances:
            like = self._item_like(instance)
            if like is not None:
                out.append((instance, like))
        return out

    @staticmethod
    def _item_like(instance):
        try:
            point = instance.Location.Point
            x_axis = instance.GetTransform().BasisX
            length = (x_axis.X ** 2 + x_axis.Y ** 2) ** 0.5
            if length == 0:
                return None
            return {
                "x_mm": point.X * geo.MM_PER_FT,
                "y_mm": point.Y * geo.MM_PER_FT,
                "facing_x": x_axis.X / length,
                "facing_y": x_axis.Y / length,
                "family_name": str(instance.Symbol.FamilyName),
                "type_name": name_of(instance.Symbol),
            }
        except Exception:
            return None

    def _tag_view(self, task, view, scale, targets, level, number, notes, room):
        rules = task.get("tag_rules") or []
        if not rules:
            return
        if not scale:
            notes.append("sheet {}: the view scale is not known, so no tags were placed".format(number))
            return
        made = [0]
        room_made = [0]
        problems = []
        other_kinds = set()
        for rule in rules:
            if not geo.is_independent(rule) and rule.get("tag_kind") != "room":
                other_kinds.add(rule.get("tag_kind"))

        def work():
            for instance, like in targets:
                for rule in geo.rules_for_item(rules, like, geo.PLAN):
                    placement = geo.tag_placement(like, rule, scale)
                    if placement is None:
                        continue
                    try:
                        self._place_tag(view, instance, rule, placement, level)
                        made[0] += 1
                    except Exception as e:
                        problems.append("tag '{}' on '{}' not placed: {}".format(rule["key"]["tag_type"], like["type_name"], e))
            for rule in geo.room_rules(rules, geo.PLAN):
                try:
                    point = room.Location.Point
                    placement = geo.tag_placement(geo.room_like(point.X * geo.MM_PER_FT, point.Y * geo.MM_PER_FT), rule, scale)
                    if placement is None:
                        continue
                    self._place_room_tag(view, room, rule, placement, level)
                    room_made[0] += 1
                except Exception as e:
                    problems.append("room tag '{}' not placed: {}".format(rule["key"]["tag_type"], e))

        self._in_transaction("RoomMate: tags for sheet {}".format(number), work)
        notes.append("sheet {}: {} tag(s) placed, {} room tag(s)".format(number, made[0], room_made[0]))
        self._capped(notes, "tags not placed", problems)
        if other_kinds:
            notes.append("tags of kind {} are not placed yet".format(", ".join(sorted(str(k) for k in other_kinds))))

    def _place_tag(self, view, instance, rule, placement, level):
        z = level.Elevation
        reference = Reference(instance)
        head = XYZ(geo.mm_to_ft(placement["head"][0]), geo.mm_to_ft(placement["head"][1]), z)
        orientation = getattr(TagOrientation, str(rule["placement"].get("orientation") or "Horizontal"), TagOrientation.Horizontal)
        tag = IndependentTag.Create(self.doc, view.Id, reference, placement["has_leader"], TagMode.TM_ADDBY_CATEGORY, orientation, head)
        key = rule["key"]
        wanted = self.facts.symbol_for(key["tag_family"], key["tag_type"])
        if wanted is not None and element_id_str(wanted.Id) != element_id_str(tag.GetTypeId()):
            tag.ChangeTypeId(wanted.Id)
        if placement["has_leader"] and placement["end"] is not None:
            try:
                tag.LeaderEndCondition = LeaderEndCondition.Free
                tag.SetLeaderEnd(reference, XYZ(geo.mm_to_ft(placement["end"][0]), geo.mm_to_ft(placement["end"][1]), z))
                if placement["elbow"] is not None:
                    tag.SetLeaderElbow(reference, XYZ(geo.mm_to_ft(placement["elbow"][0]), geo.mm_to_ft(placement["elbow"][1]), z))
            except Exception:
                # The tag is made; a leader that will not take its free end keeps Revit's.
                pass
        return tag

    def _place_room_tag(self, view, room, rule, placement, level):
        """The room's tag in `view`: made on the room's own point, then its head moved to the
        rule's place (a room tag made away from its room can be refused), with a leader back to
        the room when the rule has one."""
        z = level.Elevation
        point = room.Location.Point
        tag = self.doc.Create.NewRoomTag(LinkElementId(room.Id), UV(point.X, point.Y), view.Id)
        key = rule["key"]
        wanted = self.facts.room_tag_type(key["tag_family"], key["tag_type"])
        if wanted is not None and element_id_str(wanted.Id) != element_id_str(tag.GetTypeId()):
            tag.ChangeTypeId(wanted.Id)
        try:
            if placement["has_leader"]:
                tag.HasLeader = True
            tag.TagHeadPosition = XYZ(geo.mm_to_ft(placement["head"][0]), geo.mm_to_ft(placement["head"][1]), z)
        except Exception:
            # The tag is made on the room; a head that will not move stays where Revit put it.
            pass
        return tag

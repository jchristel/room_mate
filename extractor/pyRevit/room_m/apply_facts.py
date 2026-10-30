"""What the open Revit model can say about a task: the `facts` that `room_m.task_plan` asks.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

Read only. Nothing here changes the model; `room_m.apply_write` does that. The answers are
cached for the life of one run, because a run is one sitting over one model and the walks
(every furniture instance, every symbol) are the slow part.

**One thing is configured here and is the person's to set, not guessed:**

- `FAMILY_LIBRARY_DIR` (or the environment variable `ROOMMATE_FAMILY_LIBRARY`): a folder of
  `<family name>.rfa` files. A family that is not in the model is looked for there and loaded
  when a task needs it. Unset means the model is the only source.

**Not yet run in Revit.**
"""

import os

from Autodesk.Revit.DB import (
    Element,
    FamilySymbol,
    FilteredElementCollector,
)

from duHast.Revit.Rooms.rooms import get_all_placed_rooms

from room_m.utils.generic import element_id_str, phase_by_name
from room_m.utils.room_refs import room_reference

FAMILY_LIBRARY_DIR = os.environ.get("ROOMMATE_FAMILY_LIBRARY") or None


def name_of(element):
    """An element's name. `Element.Name` is a property that IronPython sometimes cannot reach
    on a type element, so the class-level getter is the fallback."""
    try:
        return str(element.Name)
    except Exception:
        try:
            return str(Element.Name.GetValue(element))
        except Exception:
            return ""


def _key(family, type_name):
    return ((family or "").strip().lower(), (type_name or "").strip().lower())


class RevitFacts(object):
    def __init__(self, doc, library_dir=None):
        self.doc = doc
        self.library_dir = library_dir or FAMILY_LIBRARY_DIR
        self._symbols = None
        self._rooms = None
        self._contents = {}

    # -- phases and rooms -------------------------------------------------------------------

    def phase_exists(self, phase_name):
        return any(p.Name == phase_name for p in self.doc.Phases)

    def _placed_rooms(self):
        if self._rooms is None:
            self._rooms = get_all_placed_rooms(self.doc)
        return self._rooms

    def find_room(self, number, room_id):
        """The placed room with this `Number`. Two rooms sharing a number are told apart by the
        id the task carries; if that does not settle it, the room is not found (writing into a
        guessed room is worse than stopping)."""
        rooms = self._placed_rooms()
        if number:
            matches = [r for r in rooms if str(r.Number) == str(number)]
            if len(matches) == 1:
                return matches[0]
            if len(matches) > 1:
                by_id = [r for r in matches if element_id_str(r.Id) == str(room_id)]
                return by_id[0] if len(by_id) == 1 else None
            return None
        if room_id is not None:
            by_id = [r for r in rooms if element_id_str(r.Id) == str(room_id)]
            return by_id[0] if by_id else None
        return None

    def room_items(self, room, phase_name):
        """The FF&E instances in `room` in `phase_name`: the ones a person would call its
        content. The range test for the phase, as for every element but a room itself."""
        if phase_name not in self._contents:
            self._contents[phase_name] = self._walk_contents(phase_name)
        return list(self._contents[phase_name].get(element_id_str(room.Id), []))

    def _walk_contents(self, phase_name):
        from duHast.Revit.Family.Export.to_data_item import DEFAULT_ITEM_CATEGORIES
        from Autodesk.Revit.DB import ElementCategoryFilter, FamilyInstance
        from room_m.utils.items import items_in_phase

        phase = phase_by_name(self.doc, phase_name)
        allowed = items_in_phase(self.doc, phase_name, DEFAULT_ITEM_CATEGORIES)
        by_room = {}
        for category in DEFAULT_ITEM_CATEGORIES:
            collector = FilteredElementCollector(self.doc).OfClass(FamilyInstance).WherePasses(ElementCategoryFilter(category))
            for instance in collector:
                try:
                    if element_id_str(instance.Id) not in allowed:
                        continue
                    room_id = room_reference(instance, phase, "Room")
                except Exception:
                    continue
                if room_id is not None:
                    by_room.setdefault(room_id, []).append(instance)
        return by_room

    def room_has_content(self, room, phase_name):
        return len(self.room_items(room, phase_name)) > 0

    # -- families ---------------------------------------------------------------------------

    def _symbol_map(self):
        if self._symbols is None:
            found = {}
            for symbol in FilteredElementCollector(self.doc).OfClass(FamilySymbol):
                try:
                    found.setdefault(_key(symbol.FamilyName, name_of(symbol)), symbol)
                except Exception:
                    continue
            self._symbols = found
        return self._symbols

    def symbol_for(self, family_name, type_name):
        return self._symbol_map().get(_key(family_name, type_name))

    def library_path(self, family_name):
        if not self.library_dir or not family_name:
            return None
        path = os.path.join(self.library_dir, family_name + ".rfa")
        return path if os.path.isfile(path) else None

    def family_available(self, family_name, type_name):
        if self.symbol_for(family_name, type_name) is not None:
            return True
        return self.library_path(family_name) is not None

    def forget_symbols(self):
        """After a family is loaded the cached map is stale."""
        self._symbols = None

    def refresh(self):
        """After a task wrote (or was rolled back) what was read about the model is stale: the
        symbols, the room contents."""
        self._symbols = None
        self._contents = {}

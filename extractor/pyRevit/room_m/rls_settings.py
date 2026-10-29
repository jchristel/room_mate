"""Room layout sheet settings: what the model holds, what it offers, and what is still missing.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. The Revit calls
are made inside the functions that need them, so the pure half (`missing`) is tested on CPython
(`extractor/tests/test_rls_settings.py`).

**The model is the source of truth.** The settings live in the model's extensible storage under the key
`rls` (`room_m.project_settings`) and are edited in a window (`room_m.rls_settings_window`, drawn from
`rls_settings.xaml`, its fields in `room_m.rls_settings_form`). `collect_options` reads the choices only
Revit knows (title blocks, view types, tags ...) for that window. A list that cannot be read is left out,
not sent empty: the window then says Revit offered no list, which is different from a model that has none.

**Not yet run in Revit.**
"""

PROJECT_KEY = "rls"
SIZES = ("a0", "a1", "a2", "a3")


def missing(settings):
    """What stops the settings being used, in words; empty when complete. The same question the
    window asks (`rls_settings_form.problems`): the tags are left out because without them a task only warns."""
    s = settings or {}
    sizes = s.get("sheet_sizes") if isinstance(s.get("sheet_sizes"), dict) else {}
    out = []
    for size in SIZES:
        if not sizes.get(size):
            out.append("title block for {}".format(size.upper()))
    for key, label in (
        ("plan_view_type", "plan view type"),
        ("elevation_view_type", "elevation view type"),
        ("schedule_base", "schedule base"),
        ("sheet_number", "sheet number logic"),
        ("sheet_name", "sheet name logic"),
        ("view_name", "view name logic"),
    ):
        value = s.get(key)
        if not (isinstance(value, (type(u""), str)) and value.strip()):
            out.append(label)
    return out


def _names(elements, label):
    seen = set()
    for element in elements:
        try:
            name = label(element)
        except Exception:
            continue
        if name:
            seen.add(name)
    return sorted(seen)


def collect_options(doc):
    """The choices this model offers, as the page's `options`. Each list is read on its own, and
    one that fails is left out."""
    from Autodesk.Revit.DB import (
        BuiltInCategory, ElementId, FilteredElementCollector, ViewFamily, ViewFamilyType, ViewSchedule)
    from duHast.Revit.Family.family_symbol_lookup import get_symbol_full_names_by_category

    options = {}

    def view_types(family):
        return [v for v in FilteredElementCollector(doc).OfClass(ViewFamilyType) if v.ViewFamily == family]

    def room_schedules():
        rooms = ElementId(BuiltInCategory.OST_Rooms)
        return [
            v for v in FilteredElementCollector(doc).OfClass(ViewSchedule)
            if not v.IsTemplate and not v.IsInternalKeynoteSchedule and not v.IsTitleblockRevisionSchedule
            and v.Definition.CategoryId == rooms
        ]

    def room_parameters():
        room = FilteredElementCollector(doc).OfCategory(BuiltInCategory.OST_Rooms).WhereElementIsNotElementType().FirstElement()
        return [p.Definition.Name for p in room.Parameters] if room is not None else []

    for key, read in (
        ("title_blocks", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_TitleBlocks)),
        ("plan_view_types", lambda: _names(view_types(ViewFamily.FloorPlan), lambda v: v.Name)),
        ("elevation_view_types", lambda: _names(view_types(ViewFamily.Elevation), lambda v: v.Name)),
        ("schedules", lambda: _names(room_schedules(), lambda v: v.Name)),
        ("room_tags", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_RoomTags)),
        ("independent_tags", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_MultiCategoryTags)),
        ("room_parameters", lambda: sorted(set(room_parameters()))),
    ):
        try:
            options[key] = read()
        except Exception:
            pass
    return options

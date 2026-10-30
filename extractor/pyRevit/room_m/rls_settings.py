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

from room_m.textutil import non_blank_text

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
        if non_blank_text(value) is None:
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
    from Autodesk.Revit.DB import BuiltInCategory, ViewFamily
    from duHast.Revit.Family.family_symbol_lookup import get_symbol_full_names_by_category
    from duHast.Revit.Rooms.rooms import get_room_parameter_names
    from duHast.Revit.Views.schedules import get_schedules_by_built_in_category
    from duHast.Revit.Views.views import get_view_family_types

    options = {}

    def room_schedules():
        return [
            v for v in get_schedules_by_built_in_category(doc, BuiltInCategory.OST_Rooms)
            if not v.IsTemplate and not v.IsInternalKeynoteSchedule
        ]

    for key, read in (
        ("title_blocks", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_TitleBlocks)),
        ("plan_view_types", lambda: _names(get_view_family_types(doc, ViewFamily.FloorPlan), lambda v: v.Name)),
        ("elevation_view_types", lambda: _names(get_view_family_types(doc, ViewFamily.Elevation), lambda v: v.Name)),
        ("schedules", lambda: _names(room_schedules(), lambda v: v.Name)),
        ("room_tags", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_RoomTags)),
        ("independent_tags", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_MultiCategoryTags)),
        ("room_parameters", lambda: get_room_parameter_names(doc)),
    ):
        try:
            options[key] = read()
        except Exception:
            pass
    return options

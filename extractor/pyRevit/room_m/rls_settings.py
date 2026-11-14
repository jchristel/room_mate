"""Room layout sheet settings: what the model holds, what it offers, and what is still missing.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. The Revit calls
are made inside the functions that need them, so the pure half (`missing`, `validate`) is tested on CPython
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

# The view name logic, one per kind of view (kind, settings key, what to call it, whether every sheet needs it).
# Each takes fixed text, {Room Parameter} and {Seq_Counter} (the view's place among the views of its kind, from 1).
# The 3D one is needed only by a template with a 3D viewport, so the planner asks for it, as for the 3D view type.
VIEW_NAMES = (
    ("plan", "view_name_plan", "plan view name logic", True),
    ("elevation", "view_name_elevation", "elevation view name logic", True),
    ("three_d", "view_name_three_d", "3D view name logic", False),
    ("schedule", "view_name_schedule", "schedule view name logic", True),
)
# Before 2026-11-08 one `view_name` served every kind and each view added its kind to it. A model saved then reads as
# that name plus the old ending, per kind, until the window saves it again.
# The templates of a SET of rooms (multi-room RLS, `room_m.rls_set_plan`): (settings key, the single-room key it falls
# back to when empty, what to call it). They read the LEAD room's parameters, `{Rooms:Parameter}` (every room's) and,
# the sheet number and name only, `{Sheet_Counter}`. A set's elevations and schedules keep the single-room templates,
# filled from their own room, so they are not here.
SET_TEMPLATES = (
    ("set_sheet_number", "sheet_number", "set sheet number logic"),
    ("set_sheet_name", "sheet_name", "set sheet name logic"),
    ("set_view_name_plan", "view_name_plan", "set plan view name logic"),
    ("set_view_name_three_d", "view_name_three_d", "set 3D view name logic"),
    ("set_title_on_sheet_plan", "title_on_sheet_plan", "set plan title on sheet"),
    ("set_title_on_sheet_three_d", "title_on_sheet_three_d", "set 3D title on sheet"),
)
_SET_SHEET_KEYS = ("set_sheet_number", "set_sheet_name")


def set_template(settings, key):
    """The template a set uses for `key` (one of `SET_TEMPLATES`): its own when not empty, else the single-room one."""
    s = with_view_names(settings)
    for own, fallback, _label in SET_TEMPLATES:
        if own == key:
            return (non_blank_text(s.get(own)) or non_blank_text(s.get(fallback)) or "").strip()
    raise KeyError(key)


LEGACY_VIEW_NAME = "view_name"
_LEGACY_ENDINGS = {"plan": " PLAN", "elevation": " ELEV {Seq_Counter}", "three_d": " 3D", "schedule": " SCHEDULE"}


def with_view_names(settings):
    """`settings` (a copy) with each kind's view name logic filled from the old single `view_name` where it has
    none of its own. The window shows these, and a model whose settings were saved before reads the same."""
    s = dict(settings or {})
    base = non_blank_text(s.get(LEGACY_VIEW_NAME))
    if base is not None:
        for kind, key, _label, _needed in VIEW_NAMES:
            if non_blank_text(s.get(key)) is None:
                s[key] = base.strip() + _LEGACY_ENDINGS[kind]
    return s


def missing(settings):
    """What stops the settings being used, in words; empty when complete. The same question the
    window asks (`rls_settings_form.problems`): the tags are left out because without them a task only warns."""
    s = with_view_names(settings)
    sizes = s.get("sheet_sizes") if isinstance(s.get("sheet_sizes"), dict) else {}
    out = []
    # (the 3D view type and the viewport type are not here: a task with a 3D view needs the first, the second is optional)
    # one size in use through the whole project is ordinary: a task for a size with none aborts on its own
    if not any(sizes.get(size) for size in SIZES):
        out.append("a title block for at least one sheet size")
    for key, label in (
        ("plan_view_type", "plan view type"),
        ("elevation_view_type", "elevation view type"),
        ("schedule_base", "schedule base"),
        ("sheet_number", "sheet number logic"),
        ("sheet_name", "sheet name logic"),
    ) + tuple((key, label) for _kind, key, label, needed in VIEW_NAMES if needed):
        value = s.get(key)
        if non_blank_text(value) is None:
            out.append(label)
    return out


# (settings path, the option list that must hold it, what to call it, whether a stored value not in the model stops a task)
_CHECKED = [
    (("sheet_sizes", "a0"), "title_blocks", "title block for A0", True),
    (("sheet_sizes", "a1"), "title_blocks", "title block for A1", True),
    (("sheet_sizes", "a2"), "title_blocks", "title block for A2", True),
    (("sheet_sizes", "a3"), "title_blocks", "title block for A3", True),
    (("plan_view_type",), "plan_view_types", "plan view type", True),
    (("elevation_view_type",), "elevation_view_types", "elevation view type", True),
    (("three_d_view_type",), "three_d_view_types", "3D view type", True),
    (("viewport_type",), "viewport_types", "viewport type", True),
    (("schedule_base",), "schedules", "base schedule", True),
    # without a tag a task only warns, so a stale tag does too
    (("room_tag",), "room_tags", "room tag", False),
    (("independent_tag",), "independent_tags", "independent tag", False),
    # without the 2D grid family the elevations are made without one, so a stale one only warns too
    (("elevation_grid",), "line_detail_families", "2D grid family", False),
]
_TEMPLATES = (("sheet_number", "sheet number logic"), ("sheet_name", "sheet name logic"))


def validate(settings, options):
    """`(errors, warnings)`: every stored setting the model cannot honour, in words.

    `missing` asks what is not chosen; this asks what IS chosen and is wrong: a title block, view type,
    schedule or tag the model does not have (`options` are what `collect_options` read), a template word
    that is not a room parameter, a numbering style that is not one of the two. **An option list the model
    did not offer is not checked** (the window says Revit could not list it): that is "cannot tell", never a
    finding. Errors stop a task; warnings (the tags) only warn, as a task does without them. Asked when the
    settings window opens and changes, and before the Tasks button claims anything."""
    from room_m import detail_numbers, rls_settings_form

    s = with_view_names(settings)
    errors, warnings = [], []
    for path, list_key, label, blocking in _CHECKED:
        node = s
        for key in path:
            node = node.get(key) if isinstance(node, dict) else None
        if non_blank_text(node) is None or list_key not in (options or {}):
            continue
        if node not in options[list_key]:
            (errors if blocking else warnings).append("{} '{}' is not in this model".format(label, node))
    for key, label in _TEMPLATES:
        bad = rls_settings_form.unknown_tokens(s.get(key), (options or {}).get("room_parameters"))
        if bad:
            errors.append("{}: not a room parameter: {}".format(label, " ".join("{" + t + "}" for t in bad)))
    for _kind, key, label, _needed in VIEW_NAMES:
        bad = rls_settings_form.unknown_tokens(
            s.get(key), (options or {}).get("room_parameters"), (rls_settings_form.SEQ_COUNTER,))
        if bad:
            errors.append("{}: not a room parameter: {}".format(label, " ".join("{" + t + "}" for t in bad)))
    # a sheet has several elevations, and two views cannot share a name
    elevation = non_blank_text(s.get("view_name_elevation"))
    if elevation is not None and not rls_settings_form.has_seq_counter(elevation):
        errors.append("elevation view name logic needs {Seq_Counter}: every elevation on a sheet needs its own name")
    for _name, key, kind in rls_settings_form.TITLE_TEXTS:
        bad = rls_settings_form.unknown_tokens(
            s.get(key), (options or {}).get("room_parameters"), (rls_settings_form.SEQ_COUNTER,))
        if bad:
            errors.append("{} title on sheet: not a room parameter: {}".format(
                kind.replace("three_d", "3D").replace("_", " "), " ".join("{" + t + "}" for t in bad)))
    for key in detail_numbers.SETTING_OF.values():
        if s.get(key) is not None and s.get(key) not in detail_numbers.STYLES:
            errors.append("{} '{}' is not numeric or alphabetic".format(key.replace("_", " "), s.get(key)))
    _validate_sets(s, options, errors, warnings)
    return errors, warnings


def _validate_sets(s, options, errors, warnings):
    """The set templates' own words (`SET_TEMPLATES`): a room parameter, `{Rooms:Parameter}` of one, and in the sheet
    number and name `{Sheet_Counter}`. `{Seq_Counter}` means nothing in a set's plan or 3D (one of each per set). A set
    sheet number of its own without `{Sheet_Counter}` only warns: a set of one sheet needs none, a larger one aborts."""
    from room_m import rls_settings_form
    from room_m.rls_template import SEQ_COUNTER, SHEET_COUNTER, TOKEN

    for own, _fallback, label in SET_TEMPLATES:
        text = s.get(own)
        if non_blank_text(text) is None:
            continue
        bad = rls_settings_form.unknown_set_tokens(
            text, (options or {}).get("room_parameters"), own in _SET_SHEET_KEYS)
        if bad:
            errors.append("{}: not a room parameter: {}".format(label, " ".join("{" + t + "}" for t in bad)))
        if own not in _SET_SHEET_KEYS and any(t.strip().lower() == SEQ_COUNTER.lower() for t in TOKEN.findall(text)):
            errors.append("{}: {{{}}} means nothing in it: a set has one plan and one 3D view".format(label, SEQ_COUNTER))
    # only a set number of its own: a project that never makes sets falls back and is not nagged (the planner aborts)
    number = non_blank_text(s.get("set_sheet_number"))
    if number and not any(t.strip().lower() == SHEET_COUNTER.lower() for t in TOKEN.findall(number)):
        warnings.append("set sheet number logic has no {{{}}}: a set of more than one sheet aborts".format(SHEET_COUNTER))


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


def is_line_based_detail(symbol):
    """True when `symbol` is a type of a line based detail family (`FamilyPlacementType.CurveBasedDetail`), the only kind
    duHast's `place_a_line_based_detail_family_instance_in_view` places. Revit call (made here so the pure half of this
    module imports without it)."""
    from Autodesk.Revit.DB import FamilyPlacementType

    try:
        return symbol.Family.FamilyPlacementType == FamilyPlacementType.CurveBasedDetail
    except Exception:
        return False


def line_detail_family_names(doc):
    """The "Family : Type" names of the model's line based detail families (detail components drawn along a line),
    the choices for the elevation's 2D grid. Sorted, each once."""
    from Autodesk.Revit.DB import BuiltInCategory
    from duHast.Revit.Family.family_symbol_lookup import get_family_symbols_by_category, get_symbol_full_name

    names = set()
    for symbol in get_family_symbols_by_category(doc, BuiltInCategory.OST_DetailComponents):
        try:
            if is_line_based_detail(symbol):
                names.add(get_symbol_full_name(symbol))
        except Exception:
            continue  # a type whose name cannot be read is not offered
    return sorted(names)


def first_filter_is_room_number(schedule):
    """True when the schedule's FIRST filter is on the room number. **Judged by the built-in parameter, never
    by a name**: the field's `ParameterId` is `BuiltInParameter.ROOM_NUMBER` whatever language Revit runs in,
    and whether the schedule lists rooms or, as a multi-category schedule can, anything with a room's number.
    Revit calls only (made here so the pure half of this module imports without it)."""
    from Autodesk.Revit.DB import BuiltInParameter
    from duHast.Revit.Common.element_id import get_el_id_int

    definition = schedule.Definition
    if definition.GetFilterCount() < 1:
        return False
    field = definition.GetField(definition.GetFilter(0).FieldId)
    return get_el_id_int(field.ParameterId) == int(BuiltInParameter.ROOM_NUMBER)


# how many viewports are looked at for their valid types: one per kind of view is enough, this only bounds a huge model
_VIEWPORTS_LOOKED_AT = 500


def viewport_types(doc):
    """The viewport types of the model ('Title w Line', 'No Title'), as element types.

    A viewport type has NO category (so a collector by category finds none, measured in Revit): the way to list
    them is to ask a viewport which types it accepts. What a viewport accepts can depend on its view (a section
    offers 'Section 1'), so the answer is the union over the first viewport of each kind of view (plan, elevation,
    section, 3D, legend ...). A model with no viewport (no sheet has a view on it) lists none."""
    from Autodesk.Revit.DB import FilteredElementCollector, Viewport

    kinds = set()
    seen = set()
    found = []
    for n, viewport in enumerate(FilteredElementCollector(doc).OfClass(Viewport)):
        if n >= _VIEWPORTS_LOOKED_AT:
            break
        view = doc.GetElement(viewport.ViewId)
        kind = view.ViewType if view is not None else None
        if kind in kinds:
            continue
        kinds.add(kind)
        for type_id in viewport.GetValidTypes():
            key = str(type_id)
            element = doc.GetElement(type_id)
            if key not in seen and element is not None:
                seen.add(key)
                found.append(element)
    return found


def collect_options(doc):
    """The choices this model offers, as the page's `options`. Each list is read on its own, and
    one that fails is left out."""
    from Autodesk.Revit.DB import BuiltInCategory, Element, ViewFamily
    from duHast.Revit.Family.family_symbol_lookup import get_symbol_full_names_by_category
    from duHast.Revit.Rooms.rooms import get_room_parameter_names
    from duHast.Revit.Views.schedules import get_schedules
    from duHast.Revit.Views.views import get_view_family_types

    options = {}
    errors = []

    def name_of(element):
        # `element.Name` fails on Revit types under IronPython; Element.Name reads it
        return Element.Name.GetValue(element)

    def room_number_schedules():
        """Any schedule, of one category or several, filtered first on the room number."""
        found = []
        for v in get_schedules(doc):
            try:
                if not v.IsTemplate and not v.IsInternalKeynoteSchedule and first_filter_is_room_number(v):
                    found.append(v)
            except Exception:
                continue  # a schedule that cannot be read is not offered
        return found

    for key, read in (
        ("title_blocks", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_TitleBlocks)),
        ("plan_view_types", lambda: _names(get_view_family_types(doc, ViewFamily.FloorPlan), name_of)),
        ("elevation_view_types", lambda: _names(get_view_family_types(doc, ViewFamily.Elevation), name_of)),
        ("viewport_types", lambda: _names(viewport_types(doc), name_of)),
        ("three_d_view_types", lambda: _names(get_view_family_types(doc, ViewFamily.ThreeDimensional), name_of)),
        ("schedules", lambda: _names(room_number_schedules(), name_of)),
        ("room_tags", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_RoomTags)),
        ("independent_tags", lambda: get_symbol_full_names_by_category(doc, BuiltInCategory.OST_MultiCategoryTags)),
        ("line_detail_families", lambda: line_detail_family_names(doc)),
        ("room_parameters", lambda: get_room_parameter_names(doc)),
    ):
        try:
            options[key] = read()
        except Exception as e:
            errors.append("{}: {}".format(key, e))
    if errors:
        options["errors"] = errors
    return options

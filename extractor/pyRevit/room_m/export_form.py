"""The export window's choices: what can be ticked, what is valid, and how a level choice reaches each model. No Revit, no WPF.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so that CPython
runs it too (`extractor/tests/test_export_form.py`); the window (`export_window`) only draws it.

**One window for every export.** The entities (rooms, doors, windows, FF&E, spaces, ceilings, floors) are
check boxes, and what they share -- the server project, the phase, the models, and the whole model or
chosen levels -- is asked once. A level choice is by NAME across the selected models (levels with the same
name are the same storey, as elsewhere in the project) and becomes, per model, a `scope` of that model's own
level ids, which is what the server's level-scoped push takes (`scoped_push`).

**A model that has none of the chosen levels is left out** of a scoped run and said so, rather than being
pushed unscoped (which would replace its whole snapshot with nothing).
"""

# (control name, entity key as `room_mate.ENTITY_EXPORTERS`, label)
ENTITIES = [
    ("cb_rooms", "rooms", "Rooms"),
    ("cb_doors", "doors", "Doors"),
    ("cb_windows", "windows", "Windows"),
    ("cb_ffe", "ffe", "FF&E (identified by room)"),
    ("cb_ffe_by_space", "ffe_by_space", "FF&E (identified by space: services models)"),
    ("cb_spaces", "spaces", "Spaces"),
    ("cb_ceilings", "ceilings", "Ceilings"),
    ("cb_floors", "floors", "Floors"),
]


# (control name, entity key, ticked by default): "Include shared nested families", one box beside each entity that
# has a nested rule. Only a SHARED nested family is an instance of its own in the project, so it is the only kind
# an export can see. The defaults are what the producer did before the boxes existed:
# - doors and windows drop an instance nested in another of the same category (`utils.openings.nested_opening_ids`:
#   a leaf, a panel, glazing; 55% of one facade file's "doors"), so they start unticked;
# - FF&E pushed every component and left the decision to the server's `[ffe] nested_components` at read time, so it
#   starts ticked. Unticked drops every item with a super component before it is sent, and then the server cannot
#   see it at all: a create-RLS task cannot tag a nested part (a shower rail) the library has a rule for.
# Whichever is chosen, each host still carries its `sub_components` (`utils.nesting`).
NESTED = [
    ("cb_nested_doors", "doors", False),
    ("cb_nested_windows", "windows", False),
    ("cb_nested_ffe", "ffe", True),
    ("cb_nested_ffe_by_space", "ffe_by_space", True),
]

NESTED_LABEL = "Include Shared Nested Families"


def nested_default():
    """The entity keys whose nested families are included when nothing says otherwise (the single-entity buttons,
    a first run of the window)."""
    return set(key for _name, key, on in NESTED if on)


def nested_ticked(ticked):
    """The entity keys whose "Include shared nested families" box is ticked. `ticked` is the set of control names."""
    return set(key for name, key, _on in NESTED if name in ticked)


def apply_nested(entity, contribution, include):
    """Make one model's `contribution` to `entity`'s bucket say what the nested choice asked for; the push step
    (`is_left_out_nested`, below) drops what it names. Doors and windows always read their nested ids, so
    including them is forgetting the set; FF&E reads none, so leaving them out is a flag that drops any item whose
    `super_component_id` is set. An entity with no nested rule is left alone."""
    if contribution is None:
        return contribution
    if entity in ("doors", "windows"):
        if include:
            contribution["nested_ids"] = set()
    elif entity in ("ffe", "ffe_by_space"):
        contribution["drop_components"] = not include
    return contribution


def is_left_out_nested(element, contribution, nested_ids):
    """Whether the run's nested choice leaves `element` out: an opening the exporter named as nested in another
    of its category (`nested_ids`, emptied when the export window includes them), or an FF&E component when the
    window asked for none (`drop_components`, set by `export_form.apply_nested`)."""
    if element["id"] in nested_ids:
        return True
    return bool(contribution.get("drop_components")) and bool(element.get("super_component_id"))


def entities_ticked(ticked):
    """The entity keys ticked, in the window's order. `ticked` is the set of control names that are
    ticked."""
    return tuple(key for name, key, _label in ENTITIES if name in ticked)


def entities_all():
    """The control names `Check all` ticks. FF&E by space is left out: it is the services-model alternative to FF&E by
    room and the two are never both wanted (`problems` refuses both), so ticking everything must not tick both."""
    return set(name for name, key, _label in ENTITIES if key != "ffe_by_space")


def default_phase(phases, server_phases):
    """`(phase, reason)`: the phase to pre-select, and a line that says why.

    `server_phases` are the phases the server holds for the selected models in the chosen project (a model's
    phase is fixed by its first push, so this is what the next push is checked against; `room_m.phase_hint`
    reads them). The first of `phases` (the window's list, oldest first) the server holds wins; with none, this is
    the project's first push for these models and the first phase is offered. `phase` is None only when there are
    no phases at all."""
    if not phases:
        return None, ""
    held = set(p for p in server_phases if p)
    for phase in phases:
        if phase in held:
            return phase, "Phase from the last push to this project."
    if held:
        return phases[0], "The server holds {} for these models, which this model does not have.".format(
            ", ".join(sorted(held)))
    return phases[0], "First push to this project: the first phase is offered."


def remembered(saved, projects, names):
    """What the window starts with, from the saved section of `prefs`: `{"project": index or None,
    "entities": set of control names, "levels": None for the whole model or the level names still present,
    "nested": set of nested control names}`. Anything saved that no longer exists (a project removed, a level
    renamed) is dropped, never an error; a save from before the nested boxes gets their defaults."""
    saved = saved or {}
    index = None
    for i, project in enumerate(projects):
        if project.get("id") == saved.get("project"):
            index = i
    keys = set(saved.get("entities") or [])
    ticked = set(name for name, key, _label in ENTITIES if key in keys)
    levels = None
    if saved.get("scope") == "levels":
        kept = [n for n in (saved.get("levels") or []) if n in set(names)]
        levels = kept or None
    nested_keys = set(saved["nested"]) if "nested" in saved else nested_default()
    nested = set(name for name, key, _on in NESTED if key in nested_keys)
    return {"project": index, "entities": ticked, "levels": levels, "nested": nested}


def to_save(project, entities, levels, nested=None):
    """The section `prefs` keeps after an export: the project id, the entity keys, the scope and the entity keys
    whose nested families were included."""
    return {
        "project": (project or {}).get("id"),
        "entities": list(entities),
        "scope": "whole" if levels is None else "levels",
        "levels": list(levels or []),
        "nested": sorted(nested_default() if nested is None else nested),
    }


def level_names(levels_by_doc):
    """The distinct level names across models, lowest first by the lowest elevation each name has.
    `levels_by_doc` is a list of lists of `{"id", "name", "elevation"}`."""
    lowest = {}
    for levels in levels_by_doc:
        for level in levels:
            name = level["name"]
            if name not in lowest or level["elevation"] < lowest[name]:
                lowest[name] = level["elevation"]
    return sorted(lowest, key=lambda n: (lowest[n], n))


def scope_for_doc(levels, names):
    """The ids of this model's levels whose name is in `names`: its `scope`. Empty when it has none of them."""
    wanted = set(names)
    return [level["id"] for level in levels if level["name"] in wanted]


def problems(ticked, project, phase, whole_model, chosen_levels):
    """What stops the export, as lines for the status text; empty when it can run."""
    out = []
    entities = entities_ticked(ticked)
    if not entities:
        out.append("Check at least one thing to export.")
    if "ffe" in entities and "ffe_by_space" in entities:
        out.append("FF&E is identified by room OR by space, not both: they fill the same list.")
    if not project:
        out.append("Choose the project to push to.")
    if not phase:
        out.append("Choose a phase.")
    if not whole_model and not chosen_levels:
        out.append("Choose at least one level, or push the whole model.")
    return out


def summary(entities, project_name, phase, docs, names):
    """One line for the run output: what is about to be pushed and where."""
    scope = "the whole model" if not names else "level(s) " + ", ".join(names)
    return "Pushing {} from {} model(s) to '{}', phase '{}', {}".format(
        ", ".join(entities), len(docs), project_name, phase, scope)


def level_kwargs(level_ids):
    """The keyword arguments that hand a level choice to a duHast `get_all_*_data`: none when the whole
    model is wanted, so a duHast older than the level filter still works for every unscoped push (it
    would refuse the unknown `level_ids`), and `level_ids` only when a scoped push asks for it."""
    return {} if level_ids is None else {"level_ids": level_ids}

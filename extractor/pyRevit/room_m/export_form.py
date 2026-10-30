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


def entities_ticked(ticked):
    """The entity keys ticked, in the window's order. `ticked` is the set of control names that are
    ticked."""
    return tuple(key for name, key, _label in ENTITIES if name in ticked)


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
        out.append("Tick at least one thing to export.")
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

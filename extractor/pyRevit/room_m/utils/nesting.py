"""What is nested in what: each host instance's shared nested families, every depth, for the server.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

**Why the HOST carries it.** A child already names its parent on FF&E (`super_component_id`), but a nested door is
dropped by default (it is not a door) and the export window can drop FF&E components too, so the child's own link
does not reach the server whenever the child does not. A blueprint that swaps a family needs the other direction
whatever was pushed: which nested families come with this host. So every door, window and FF&E item is sent with
`sub_components`, a flat list of `{"id", "parent_id", "category", "family_name", "type_name"}` (the host's id is the
`parent_id` of a direct child), read here in one pass over the document's family instances.

Only a SHARED nested family is an instance in the project (`FamilyInstance.SuperComponent` is set on it); a
non-shared one exists only inside the family document and is not seen. The walk is over every category, because a
door's hardware is a Generic Model and a shower's rail may be Specialty Equipment.

`descendants` is pure and tested on CPython (`extractor/tests/test_nesting.py`); `read_nesting` is the Revit read.
"""


def descendants(children_of, info, host_id):
    """Every instance under `host_id`, depth first, as the wire's dicts. `children_of` maps a parent id to its
    child ids, `info` an id to `{"category", "family_name", "type_name"}`. A cycle (which Revit does not make) is
    walked once."""
    out = []
    seen = set([host_id])
    stack = [(host_id, c) for c in reversed(children_of.get(host_id, []))]
    while stack:
        parent, child = stack.pop()
        if child in seen:
            continue
        seen.add(child)
        entry = {"id": child, "parent_id": parent}
        for key, value in sorted((info.get(child) or {}).items()):
            if value:
                entry[key] = value
        out.append(entry)
        stack.extend((child, c) for c in reversed(children_of.get(child, [])))
    return out


def nesting_map(parent_of, info):
    """`{host id: [sub component, ...]}` for every instance that has at least one, from `parent_of` (child id ->
    parent id). Ids are the strings the wire uses."""
    children_of = {}
    for child in sorted(parent_of):
        children_of.setdefault(parent_of[child], []).append(child)
    return dict((host, descendants(children_of, info, host)) for host in children_of)


def _type_name(symbol):
    from Autodesk.Revit.DB import Element

    try:
        return Element.Name.GetValue(symbol)
    except Exception:
        return None


def read_nesting(doc):
    """`nesting_map` for `doc`: one collector over its family instances, each nested one recorded with its parent.
    A failure is an empty map, never a failed export: the nesting is extra information about elements that are
    pushed either way."""
    try:
        from Autodesk.Revit.DB import FamilyInstance, FilteredElementCollector
        from room_m.utils.generic import element_id_str

        parent_of = {}
        info = {}
        for instance in FilteredElementCollector(doc).OfClass(FamilyInstance):
            parent = instance.SuperComponent
            if parent is None:
                continue
            child_id = element_id_str(instance.Id)
            parent_id = element_id_str(parent.Id)
            parent_of[child_id] = parent_id
            symbol = instance.Symbol
            info[child_id] = {
                "category": instance.Category.Name if instance.Category is not None else None,
                "family_name": symbol.FamilyName if symbol is not None else None,
                "type_name": _type_name(symbol) if symbol is not None else None,
            }
        return nesting_map(parent_of, info)
    except Exception as e:
        print("Could not read the nested families: {}".format(e))
        return {}

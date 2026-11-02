"""Read the type code of a tagged element from Revit.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

Plan decision 27: a tag rule is keyed by the TYPE CODE of the item it tags, the value of one type
parameter the project names in its settings (`tag_type_code`). Every instance of a type shares it,
so a tag learned from one room's bed is found for the same bed type in another room. This module
only reads the value; `room_m.sheet_tags.attach_type_codes` puts it on the observations, and the
server decides what a missing one means.

A tagged element in a LINKED model (`link_element_id` is set) is not read: its type lives in
another document, and the observation then goes without a code and is skipped by the server with
a reason. **Not run in Revit.**
"""

from duHast.Revit.Common.element_id import get_element_id_from_int


def _text(parameter):
    """The parameter's value as text, empty when it has none."""
    if parameter is None or not parameter.HasValue:
        return ""
    value = parameter.AsString()
    if not value:
        value = parameter.AsValueString()
    return (value or "").strip()


def type_code_reader(doc, parameter_name):
    """A function `read(tagged_element) -> code or None` for the elements of `doc`.

    `tagged_element` is the duHast tagged-element dictionary (`host_element_id`,
    `link_element_id`). Types are cached by id, because a sheet tags the same few types many times."""
    cache = {}

    def read(tagged_element):
        if tagged_element.get("link_element_id", 0) > 0:
            return None
        element_id = tagged_element.get("host_element_id")
        if not element_id or element_id <= 0:
            return None
        element = doc.GetElement(get_element_id_from_int(element_id))
        if element is None:
            return None
        type_id = element.GetTypeId()
        key = str(type_id)
        if key not in cache:
            type_element = doc.GetElement(type_id)
            code = ""
            if type_element is not None:
                code = _text(type_element.LookupParameter(parameter_name))
            cache[key] = code or None
        return cache[key]

    return read

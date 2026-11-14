"""The `{Room Parameter}` placeholders of the RLS number and name templates. ASCII only.

One pattern, read by `room_m.rls_task` (to fill a template) and `room_m.rls_settings_form` (to say
which words are not room parameters), so the two cannot disagree on what a placeholder is."""

import re

TOKEN = re.compile(r"\{([^{}]*)\}")

# A Title on Sheet template may also name this: the viewport's place among the viewports of its kind on the sheet,
# from 1 (the first elevation is 1, the second 2; the plan, alone of its kind, 1). It is not a room parameter.
SEQ_COUNTER = "Seq_Counter"

# A set of rooms (multi-room RLS) makes several sheets: this is the sheet's place in the set, from 1. Only the set's
# sheet number and name may name it, and the sheet number must when the set has more than one sheet.
SHEET_COUNTER = "Sheet_Counter"

# `{Rooms:Number}` in a set's template: that parameter of EVERY room of the set, distinct values joined by ROOMS_JOIN.
ROOMS_PREFIX = "Rooms:"
ROOMS_JOIN = ", "


def rooms_parameter(token):
    """The parameter a `{Rooms:Parameter}` token names, or None for any other token."""
    t = (token or "").strip()
    if t.lower().startswith(ROOMS_PREFIX.lower()):
        name = t[len(ROOMS_PREFIX):].strip()
        return name or None
    return None

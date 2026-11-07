"""The `{Room Parameter}` placeholders of the RLS number and name templates. ASCII only.

One pattern, read by `room_m.rls_task` (to fill a template) and `room_m.rls_settings_form` (to say
which words are not room parameters), so the two cannot disagree on what a placeholder is."""

import re

TOKEN = re.compile(r"\{([^{}]*)\}")

# A Title on Sheet template may also name this: the viewport's place among the viewports of its kind on the sheet,
# from 1 (the first elevation is 1, the second 2; the plan, alone of its kind, 1). It is not a room parameter.
SEQ_COUNTER = "Seq_Counter"

"""The `{Room Parameter}` placeholders of the RLS number and name templates. ASCII only.

One pattern, read by `room_m.rls_task` (to fill a template) and `room_m.rls_settings_form` (to say
which words are not room parameters), so the two cannot disagree on what a placeholder is."""

import re

TOKEN = re.compile(r"\{([^{}]*)\}")

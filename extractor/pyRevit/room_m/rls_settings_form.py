"""The RLS settings as a form: which fields there are, and how a settings object maps to them. No Revit, no WPF.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Written so that CPython
runs it too (`extractor/tests/test_rls_settings_form.py`); the window (`rls_settings_window`) only draws it.

**The same settings as the web page** (`src-js/model-settings`, `room_m.rls_settings`): the keys are the
contract, and a key this form does not know rides along untouched, so a setting a later version wrote
survives this one saving.
"""

from room_m.rls_template import TOKEN
from room_m.textutil import is_text

NOT_SET = "(not set)"
NOT_IN_MODEL = " (not in this model)"
TOKEN_PROMPT = "insert room parameter ..."

# (control name, the option list in `rls_settings.collect_options`, path in the settings)
COMBOS = [
    ("cb_a0", "title_blocks", ("sheet_sizes", "a0")),
    ("cb_a1", "title_blocks", ("sheet_sizes", "a1")),
    ("cb_a2", "title_blocks", ("sheet_sizes", "a2")),
    ("cb_a3", "title_blocks", ("sheet_sizes", "a3")),
    ("cb_plan_view_type", "plan_view_types", ("plan_view_type",)),
    ("cb_elevation_view_type", "elevation_view_types", ("elevation_view_type",)),
    ("cb_schedule_base", "schedules", ("schedule_base",)),
    ("cb_room_tag", "room_tags", ("room_tag",)),
    ("cb_independent_tag", "independent_tags", ("independent_tag",)),
]

# (text box name, settings key); each has a combo of room parameters named like it with `tk_` for its prefix
TEXTS = [
    ("tb_sheet_number", "sheet_number"),
    ("tb_sheet_name", "sheet_name"),
    ("tb_view_name", "view_name"),
]


def get_path(settings, path):
    node = settings
    for key in path:
        if not isinstance(node, dict):
            return ""
        node = node.get(key)
    return node if is_text(node) else ""


def set_path(settings, path, value):
    node = settings
    for key in path[:-1]:
        if not isinstance(node.get(key), dict):
            node[key] = {}
        node = node[key]
    node[path[-1]] = value


def display_name(option):
    """How a "Family : Type" name reads in a list: "Family [Type]". Anything else as it is."""
    family, sep, type_name = option.partition(" : ")
    return "{} [{}]".format(family, type_name) if sep else option


def choices(value, options):
    """`(labels, values, selected)` for one combo: a first entry that means "not set", then what the
    model offers, and the stored value kept visible, marked, when the model no longer has it."""
    labels = [NOT_SET]
    values = [""]
    for option in options or []:
        labels.append(display_name(option))
        values.append(option)
    selected = 0
    if value:
        if value in values:
            selected = values.index(value)
        else:
            labels.append(display_name(value) + NOT_IN_MODEL)
            values.append(value)
            selected = len(values) - 1
    return labels, values, selected


def token_labels(parameters):
    """The labels of a room parameter combo: a prompt, then the parameter names."""
    return [TOKEN_PROMPT] + list(parameters or [])


def add_token(text, parameters, index):
    """`text` with `{parameter}` added at the end, for the entry `index` of `token_labels`; `text`
    as it is when the prompt (index 0) or nothing was picked."""
    if index is None or index < 1 or index > len(parameters or []):
        return text
    return "{}{{{}}}".format(text or "", parameters[index - 1])


def unknown_tokens(template, parameters):
    """The `{token}` names of `template` that are not a room parameter the model offered. Empty when
    the model offered none (a list that could not be read must not make everything look wrong)."""
    if not parameters:
        return []
    known = set(p.strip().lower() for p in parameters)
    return [t for t in TOKEN.findall(template or "") if t.strip().lower() not in known]


def problems(settings, parameters):
    """What the person should still look at, as lines for the status text: what is not chosen yet
    (`rls_settings.missing`), and any template token that is not a room parameter."""
    from room_m import rls_settings

    lines = []
    todo = rls_settings.missing(settings)
    if todo:
        lines.append("Still to choose: {}".format(", ".join(todo)))
    for _name, key in TEXTS:
        bad = unknown_tokens(settings.get(key), parameters)
        if bad:
            lines.append("{}: not a room parameter: {}".format(key.replace("_", " "), " ".join("{" + t + "}" for t in bad)))
    return lines


def to_settings(existing, combo_values, texts):
    """The settings object after the form: `existing` with each combo's value and each text set.
    `combo_values` is `{control name: value}`, `texts` is `{control name: text}`. Keys the form does
    not know are kept."""
    settings = dict(existing or {})
    settings["sheet_sizes"] = dict(settings.get("sheet_sizes") or {})
    for name, _options, path in COMBOS:
        if name in combo_values:
            set_path(settings, path, combo_values[name])
    for name, key in TEXTS:
        if name in texts:
            settings[key] = texts[name].strip()
    return settings

"""What the windows remember between runs: the last project, what was exported, the sheet filter and room rule.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one. Pure Python, so CPython runs
it (`extractor/tests/test_prefs.py`).

**A file beside `server.json`** (`RoomMate\\prefs.json` in the user's application data), one JSON object with a
section per window (`export`, `sheets`). It is a convenience and never a requirement: a missing, unreadable or
half-written file is an empty answer, and a write that fails is reported with False and never raised, so a
window that cannot remember still works. A section is replaced whole by `save_section`; the others are kept.
What is stored is what a person typed or ticked (a project id, entity keys, level names): nothing secret.
"""

import json
import os

from room_m import server_config


def path(environ=None):
    """`RoomMate\\prefs.json`, in the folder `server.json` lives in."""
    return os.path.join(os.path.dirname(server_config.config_path(environ)), "prefs.json")


def _read(where):
    try:
        with open(where, "r") as handle:
            data = json.load(handle)
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def section(name, where=None):
    """The saved choices of one window, or {} when there are none."""
    value = _read(where or path()).get(name)
    return value if isinstance(value, dict) else {}


def save_section(name, values, where=None):
    """Keep `values` (a dict) as the choices of window `name`, leaving the other windows' as they are.
    Returns whether it was written."""
    where = where or path()
    try:
        data = _read(where)
        data[name] = values
        folder = os.path.dirname(where)
        if folder and not os.path.isdir(folder):
            os.makedirs(folder)
        with open(where, "w") as handle:
            json.dump(data, handle, indent=2, sort_keys=True)
        return True
    except Exception:
        return False

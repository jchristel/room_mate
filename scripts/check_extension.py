"""Checks the pyRevit extension source in extractor/pyRevit/RoomMate.extension.

Every failure here is one that surfaces only inside Revit, hours later, as a
button that is missing or a tab that will not load -- pyRevit reports a bad
bundle by quietly not drawing it, and IronPython reports a non-ASCII source
file as a broken module. So the checks are cheap and the failure is loud:

- **ASCII, LF, trailing newline.** IronPython 2.7 refuses to parse a source
  file holding any non-ASCII byte, and fails at import with a SyntaxError
  nowhere near the character. Line endings follow .gitattributes, which is LF
  for .py/.yaml/.json.
- **Every layout name matches a bundle folder, both ways.** A name in a
  bundle.yaml layout with no folder is a button that never appears; a folder
  missing from the layout is a button pyRevit may order arbitrarily.
- **Every button imports one entry point and calls it**, and that entry point
  exists in room_m/room_mate.py. The scripts are near-identical boilerplate
  and were copied from each other, which is how one of them shipped a
  docstring about grid bubbles and another called a different function than
  its name suggested.

Run from anywhere: python scripts/check_extension.py
"""

import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXTENSION = os.path.join(REPO, "extractor", "pyRevit", "RoomMate.extension")
ROOM_MATE = os.path.join(REPO, "extractor", "pyRevit", "room_m", "room_mate.py")

# .gitattributes stores these LF; .txt is CRLF there, so it is not checked here.
LF_SUFFIXES = (".py", ".yaml", ".json")
BUNDLE_SUFFIXES = (".pushbutton", ".pulldown", ".panel")


def walk(root):
    for dirpath, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d != "__pycache__"]
        yield dirpath, files


def check_text_files(problems):
    for dirpath, files in walk(EXTENSION):
        for name in files:
            if not name.endswith(LF_SUFFIXES):
                continue
            path = os.path.join(dirpath, name)
            rel = os.path.relpath(path, EXTENSION)
            data = open(path, "rb").read()
            non_ascii = sum(1 for byte in data if byte > 127)
            if non_ascii:
                problems.append("%s: %d non-ascii bytes" % (rel, non_ascii))
            if b"\r" in data:
                problems.append("%s: CR in file, .gitattributes says LF" % rel)
            if data and not data.endswith(b"\n"):
                problems.append("%s: no trailing newline" % rel)


def check_layouts(problems):
    for dirpath, _ in walk(EXTENSION):
        bundle = os.path.join(dirpath, "bundle.yaml")
        if not os.path.isfile(bundle):
            continue
        text = open(bundle, encoding="ascii").read()
        match = re.search(r"(?m)^layout:\s*\n((?:\s*-\s*.+\n)+)", text)
        if not match:
            continue
        rel = os.path.relpath(bundle, EXTENSION)
        listed = [line.strip().lstrip("-").strip() for line in match.group(1).strip().splitlines()]
        present = {
            name.split(".")[0]
            for name in os.listdir(dirpath)
            if os.path.isdir(os.path.join(dirpath, name)) and name.endswith(BUNDLE_SUFFIXES)
        }
        for name in listed:
            if name not in present:
                problems.append("%s: layout names %r, no such bundle" % (rel, name))
        for name in sorted(present):
            if name not in listed:
                problems.append("%s: bundle %r missing from layout" % (rel, name))


def check_icons(problems):
    for dirpath, files in walk(EXTENSION):
        if not dirpath.endswith(BUNDLE_SUFFIXES):
            continue
        for icon in ("Icon.png", "Icon.dark.png"):
            if icon not in files:
                problems.append("%s: missing %s" % (os.path.relpath(dirpath, EXTENSION), icon))


def check_buttons(problems):
    """Returns the set of entry points the buttons wire up."""
    wired = set()
    defined = set(re.findall(r"(?m)^def (\w+_entry)\(", open(ROOM_MATE, encoding="utf-8").read()))
    for dirpath, files in walk(EXTENSION):
        if "script.py" not in files:
            continue
        path = os.path.join(dirpath, "script.py")
        rel = os.path.relpath(path, EXTENSION)
        source = open(path, encoding="ascii").read()
        compile(source, path, "exec")
        imported = re.findall(r"from room_m\.room_mate import (\w+)", source)
        if len(imported) != 1:
            problems.append("%s: imports %d entry points, expected 1" % (rel, len(imported)))
            continue
        entry = imported[0]
        wired.add(entry)
        if not re.search(r"(?m)^\s*(?:\w+\s*=\s*)?%s\(" % re.escape(entry), source):
            problems.append("%s: imports %s but never calls it" % (rel, entry))
        if entry not in defined:
            problems.append("%s: %s is not defined in room_mate.py" % (rel, entry))
    return wired, defined


def main():
    problems = []
    if not os.path.isdir(EXTENSION):
        print("no extension source at %s" % EXTENSION)
        return 1

    check_text_files(problems)
    check_layouts(problems)
    check_icons(problems)
    wired, defined = check_buttons(problems)

    json.load(open(os.path.join(EXTENSION, "extension.json"), encoding="ascii"))

    print("buttons wired: %d" % len(wired))
    # export_entry is the shared helper the eight entry points are one line
    # over, so it is expected here and is not a missing button.
    unused = sorted(name for name in defined - wired if name != "export_entry")
    if unused:
        print("entry points with no button: %s" % ", ".join(unused))

    if problems:
        print("PROBLEMS:")
        for problem in problems:
            print("  - %s" % problem)
        return 1
    print("no problems found")
    return 0


if __name__ == "__main__":
    sys.exit(main())

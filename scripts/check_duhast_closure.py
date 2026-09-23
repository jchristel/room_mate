"""Checks a STAGED extension's duHast against what room_m actually imports.

Run over the folder the build assembles, not over the repository: the point is
to fail on the build machine rather than inside Revit, where a missing duHast
module is an import error in a dialog and a missing DLL is worse.

Two questions, both cheap, both learned the hard way:

- **Does every duHast module room_m reaches exist in the shipped copy?** The
  extension carries duHast's python and none of its binaries, and room_m's
  import surface is a fraction of the package -- 126 of 782 files when this was
  written. Something the build left out fails at import, per button, in Revit.
- **Does any reachable module load a DLL by path?** `Utilities/file_base_*_net`
  loads FileIOWrapper.dll at IMPORT time, from a lib folder the build does not
  ship. Nothing room_m reaches does that today. The day something does, this is
  the check that says so, instead of a button that stops working on machines
  that never had duHast installed separately.

Usage: python scripts/check_duhast_closure.py <staged extension lib folder>
"""

import os
import re
import sys

# Both forms, because duHast uses each: `from duHast.X.Y import name` may name a
# MODULE rather than a symbol, so an imported name is resolved as a module too.
IMPORT_PATTERN = re.compile(
    r"^[ \t]*(?:from[ \t]+(duHast[\w.]*)[ \t]+import[ \t]+([\w, ()]+)|import[ \t]+(duHast[\w.]*))",
    re.M,
)
DLL_PATTERN = re.compile(r"AddReferenceToFileAndPath|AddReferenceByPartialName")


def resolve(lib_dir, module):
    """The file `module` would import from, or None if the copy lacks it."""
    base = os.path.join(lib_dir, *module.split("."))
    if os.path.isfile(base + ".py"):
        return base + ".py"
    init = os.path.join(base, "__init__.py")
    if os.path.isfile(init):
        return init
    return None


def imported_modules(path, lib_dir):
    source = open(path, encoding="utf-8", errors="replace").read()
    found = []
    for match in IMPORT_PATTERN.finditer(source):
        if match.group(3):
            found.append(match.group(3))
            continue
        package = match.group(1)
        found.append(package)
        for name in re.split(r"[,\s()]+", match.group(2)):
            if name and resolve(lib_dir, package + "." + name):
                found.append(package + "." + name)
    return found


def main(argv):
    if len(argv) != 2:
        print(__doc__.strip().splitlines()[-1])
        return 2
    lib_dir = os.path.abspath(argv[1])
    room_m = os.path.join(lib_dir, "room_m")
    if not os.path.isdir(room_m) or not os.path.isdir(os.path.join(lib_dir, "duHast")):
        print("expected room_m and duHast side by side in %s" % lib_dir)
        return 1

    missing = []
    dll_loaders = []
    reached = set()
    queue = []
    for dirpath, dirs, files in os.walk(room_m):
        dirs[:] = [d for d in dirs if d != "__pycache__"]
        queue.extend(os.path.join(dirpath, f) for f in files if f.endswith(".py"))

    while queue:
        path = queue.pop()
        for module in imported_modules(path, lib_dir):
            parts = module.split(".")
            # Every ancestor is imported too, so each one has to be present.
            for depth in range(1, len(parts) + 1):
                name = ".".join(parts[:depth])
                if name in reached:
                    continue
                reached.add(name)
                resolved = resolve(lib_dir, name)
                if resolved is None:
                    missing.append((name, os.path.relpath(path, lib_dir)))
                    continue
                queue.append(resolved)
                if DLL_PATTERN.search(open(resolved, encoding="utf-8", errors="replace").read()):
                    dll_loaders.append(name)

    # Reachable duHast files holding non-ASCII bytes with no encoding cookie.
    # Reported, never failed: under IronPython 2.7 each is an import-time
    # SyntaxError, but a 2025+ extension may run a python 3 engine where the
    # same file is fine. It is duHast's call to make, and this says which files
    # the call is about rather than leaving it to a Revit dialog.
    cookie_free = []
    for name in sorted(reached):
        path = resolve(lib_dir, name)
        if path is None:
            continue
        data = open(path, "rb").read()
        if not any(byte > 127 for byte in data):
            continue
        head = data[:200].decode("latin-1").splitlines()[:2]
        if not any("coding" in line for line in head):
            cookie_free.append(os.path.relpath(path, lib_dir))

    shipped = sum(
        1
        for dirpath, dirs, files in os.walk(os.path.join(lib_dir, "duHast"))
        for f in files
        if f.endswith(".py")
    )
    print("duHast modules reached by room_m: %d of %d shipped" % (len(reached), shipped))
    if cookie_free:
        print("note: %d reachable duHast files hold non-ascii bytes and no encoding cookie:"
              % len(cookie_free))
        for path in cookie_free:
            print("  - %s" % path)

    if missing:
        print("PROBLEMS: modules room_m imports are not in the shipped duHast:")
        for name, importer in sorted(set(missing)):
            print("  - %s (imported by %s)" % (name, importer))
    if dll_loaders:
        print("PROBLEMS: reachable modules load a DLL by path, which is not shipped:")
        for name in sorted(set(dll_loaders)):
            print("  - %s" % name)
    if missing or dll_loaders:
        return 1
    print("no problems found")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

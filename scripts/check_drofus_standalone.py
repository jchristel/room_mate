"""Fails if `crates/drofus-mcp` stopped being copy-out-able.

The crate exists to be forkable: the folder, copied anywhere, must build. That breaks the moment it grows a
path dependency, inherits something from the workspace, or the source imports another crate of this
repository. Cheap to check, expensive to find out later.

Usage: python scripts/check_drofus_standalone.py
"""

import os
import re
import sys

ROOT = os.path.join(os.path.dirname(__file__), "..", "crates", "drofus-mcp")
FORBIDDEN_CRATES = ("roommate", "roommate_shared")


def main():
    problems = []
    with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as f:
        # comments explain the rule and may quote what it forbids
        manifest = "\n".join(line.split("#", 1)[0] for line in f.read().splitlines())
    if re.search(r"\bpath\s*=", manifest.replace('path = "src/', "")):
        problems.append("Cargo.toml has a path dependency")
    if re.search(r"workspace\s*=\s*true", manifest):
        problems.append("Cargo.toml inherits from the workspace (workspace = true)")
    if re.search(r"roommate", manifest.split("[dependencies]", 1)[-1]):
        problems.append("Cargo.toml depends on a roommate crate")
    for dirpath, _dirs, files in os.walk(os.path.join(ROOT, "src")):
        for name in files:
            if not name.endswith(".rs"):
                continue
            with open(os.path.join(dirpath, name), encoding="utf-8") as f:
                text = f.read()
            for crate in FORBIDDEN_CRATES:
                if re.search(r"\b(use|extern crate)\s+%s\b|\b%s::" % (crate, crate), text):
                    problems.append("%s imports %s" % (os.path.join("src", name), crate))
    for line in problems:
        print("problem: " + line)
    print("no problems found" if not problems else "%d problem(s)" % len(problems))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())

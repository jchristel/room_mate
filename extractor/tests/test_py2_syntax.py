"""IronPython 2.7 runs the extension, CPython 3 runs these tests, so a Python-3-only construct passes every other
test and fails only when Revit imports the module. This catches the ones that have bitten: `print` used as a value
(`[print]`, `sink or print`), which is a statement in Python 2, and f-strings, `nonlocal`, `yield from`."""
import io
import os
import tokenize
import unittest

ROOM_M = os.path.join(os.path.dirname(__file__), "..", "pyRevit")


def sources():
    for folder, _dirs, files in os.walk(ROOM_M):
        for name in files:
            if name.endswith(".py"):
                yield os.path.join(folder, name)


class Python2Syntax(unittest.TestCase):
    def test_no_python3_only_constructs(self):
        problems = []
        for path in sources():
            with io.open(path, encoding="utf-8") as handle:
                tokens = list(tokenize.generate_tokens(handle.readline))
            for i, tok in enumerate(tokens):
                after = next((t for t in tokens[i + 1:] if t.type not in (tokenize.NL, tokenize.COMMENT)), None)
                if tok.type == tokenize.NAME and tok.string == "print" and after is not None and after.string != "(":
                    problems.append("{}:{} print used as a value".format(path, tok.start[0]))
                if tok.type == tokenize.NAME and tok.string in ("nonlocal",):
                    problems.append("{}:{} {}".format(path, tok.start[0], tok.string))
                if tok.type == tokenize.NAME and tok.string == "from" and after is not None \
                        and tokens[i - 1].string == "yield":
                    problems.append("{}:{} yield from".format(path, tok.start[0]))
                if tok.type == tokenize.STRING and tok.string[:1].lower() == "f" and tok.string[1:2] in ("'", '"'):
                    problems.append("{}:{} f-string".format(path, tok.start[0]))
        self.assertEqual([], problems)


if __name__ == "__main__":
    unittest.main()

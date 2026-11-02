"""Small text helpers shared by the pure modules. ASCII only; CPython and IronPython 2.7.

JSON text read under IronPython 2.7 is `unicode`, which is not a `str` there, so "is this a
string" is asked in one place."""

TEXT_TYPES = (str, type(u""))


def is_text(value):
    return isinstance(value, TEXT_TYPES)


def non_blank_text(value):
    """`value` when it is text with something in it, else None."""
    return value if is_text(value) and value.strip() else None

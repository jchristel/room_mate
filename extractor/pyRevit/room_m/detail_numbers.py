"""Detail numbers of the viewports on an RLS sheet. Pure: no Revit, so CPython tests it
(`extractor/tests/test_detail_numbers.py`).

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

Each kind of view (elevation, plan, 3D) has a style in the RLS settings: `numeric` (1, 2, 3 ...) or
`alphabetic` (A, B, ... Z, AA, AB ...). **Kinds that share a style share one run**, numbered in the order
elevations, plans, 3D: with all three the same the elevations take 1..n, the plan n+1 and the 3D views
after it. A kind with a style of its own starts again at 1 (or A).
"""

NUMERIC = "numeric"
ALPHABETIC = "alphabetic"
STYLES = (NUMERIC, ALPHABETIC)
DEFAULT_STYLE = NUMERIC

# the order a run is numbered in, and the settings key holding each kind's style
KINDS = ("elevation", "plan", "three_d")
SETTING_OF = {
    "elevation": "elevation_detail_numbering",
    "plan": "plan_detail_numbering",
    "three_d": "three_d_detail_numbering",
}
PLACEHOLDER = "place holder {}"


def style_of(settings, kind):
    """The style the settings name for `kind`; numeric when absent or not one of the two."""
    value = (settings or {}).get(SETTING_OF[kind])
    return value if value in STYLES else DEFAULT_STYLE


def label(style, n):
    """The `n`th (from 1) detail number of `style`: 1, 2 ... or A .. Z, AA, AB ..."""
    if style != ALPHABETIC:
        return str(n)
    text = ""
    while n > 0:
        n, rest = divmod(n - 1, 26)
        text = chr(ord("A") + rest) + text
    return text


def assign(counts, settings):
    """`{kind: [labels]}` for `counts`, `{kind: how many viewports}`: runs per style, elevations first."""
    styles = dict((kind, style_of(settings, kind)) for kind in KINDS)
    used = dict((style, 0) for style in STYLES)
    out = {}
    for kind in KINDS:
        labels = []
        for _ in range(counts.get(kind, 0)):
            used[styles[kind]] += 1
            labels.append(label(styles[kind], used[styles[kind]]))
        out[kind] = labels
    return out


def placeholders(total):
    """`total` distinct throw-away detail numbers. Revit gives a new viewport a detail number of its own
    and rejects one already used on the sheet, so every viewport is parked on one of these before the
    real numbers go on: no real number can then meet an automatic one."""
    return [PLACEHOLDER.format(i) for i in range(1, total + 1)]

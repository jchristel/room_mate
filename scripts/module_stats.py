#!/usr/bin/env python3
"""Measure every module the module plan draws, and emit (or check) its `STATS`.

**Why this exists.** `docs/module-plan.html` says a cell's width follows its
*code* lines and nothing else, so a cell means the same thing in every language
on the page. That claim is only worth as much as the numbers behind it, and
those numbers were hand-maintained: `docs/README.md` described the plan as
"generated from the module headers" while nothing generated anything. The gap
showed. By 2026-09-06 the plan carried 72 modules and had no cell at all for
windows, FF&E or spaces, and two modules had been renamed underneath it —
`service/doors.rs` to `service/openings.rs`, `utils/doors.py` to
`utils/openings.py` — leaving three dependency edges pointing at ids that no
longer existed.

**What is generated and what is not, because the split is the whole design.**
This script owns the `STATS` block: the four measured numbers per module, which
are mechanical and therefore worth nobody's afternoon. It does **not** own `M` —
the blurbs, the zones and the import edges are judgement, and a generator that
guessed them would produce a page that says less than the module headers it was
derived from. `MODULES` below is the inventory the two halves share, so a module
added to the plan without a row here fails `--check` rather than silently
drawing at whatever width it last had.

**Why the measurement is defined the way it is.** `code` is non-blank lines that
are not comments, doc comments or docstrings, with Rust's inline `#[cfg(test)]`
modules taken out. Rust's test modules are the reason: this codebase keeps its
tests inline, so `storage/fs.rs` is 1,758 lines of which 907 are tests, and a
naive line count would draw the store as the widest room on the page for
reasons that say nothing about the store. The same rule gives a `.test.ts`
sibling back to the module it tests rather than letting it stand as its own
cell.

**Trust the reproduction, not the parser.** The measurement is regex-free line
classification, not a real parser, and it will disagree with a compiler at the
margins — an odd string containing `*/`, a `//` inside a raw string. That is
acceptable *because the numbers are a width*, and it is checkable: when this
replaced the hand-maintained block, every module untouched since that block was
written reproduced its previous numbers exactly (`lib.rs`, `check_areas.py`,
the `gen_*` group). Re-run that comparison rather than trusting a rewrite of
the classifier here.

No third-party dependencies: stdlib only, so it runs anywhere the repo does.

    python scripts/module_stats.py            # print the STATS block
    python scripts/module_stats.py --check     # exit 1 if the plan disagrees
"""

import glob
import os
import re
import sys

PLAN = "docs/module-plan.html"

# id -> the file(s) that id measures. A list because some cells are deliberately
# a *group*: `gen` is four fixture generators nobody reads separately, and
# `probes` is the probe/analyse pairs, which only mean anything as pairs. The
# ids are the plan's own, so this map is also the inventory `--check` enforces.
MODULES = {
    # ---- extractor
    "room_mate": ["extractor/pyRevit/room_m/room_mate.py"],
    "ex_rooms": ["extractor/pyRevit/room_m/exporters/rooms.py"],
    "ex_doors": ["extractor/pyRevit/room_m/exporters/doors.py"],
    "ex_windows": ["extractor/pyRevit/room_m/exporters/windows.py"],
    "ex_ffe": ["extractor/pyRevit/room_m/exporters/ffe.py"],
    "ex_spaces": ["extractor/pyRevit/room_m/exporters/spaces.py"],
    "post_common": ["extractor/pyRevit/room_m/post_common.py"],
    "post_entity": ["extractor/pyRevit/room_m/post_entity.py"],
    "post_rooms": ["extractor/pyRevit/room_m/post_rooms.py"],
    "post_doors": ["extractor/pyRevit/room_m/post_doors.py"],
    "post_windows": ["extractor/pyRevit/room_m/post_windows.py"],
    "post_ffe": ["extractor/pyRevit/room_m/post_ffe.py"],
    "post_spaces": ["extractor/pyRevit/room_m/post_spaces.py"],
    "u_envelope": ["extractor/pyRevit/room_m/utils/post_envelope.py"],
    "u_phase": ["extractor/pyRevit/room_m/utils/phase_filter.py"],
    "u_rooms": ["extractor/pyRevit/room_m/utils/rooms.py"],
    "u_openings": ["extractor/pyRevit/room_m/utils/openings.py"],
    "u_items": ["extractor/pyRevit/room_m/utils/items.py"],
    "u_spaces": ["extractor/pyRevit/room_m/utils/spaces.py"],
    "u_refs": ["extractor/pyRevit/room_m/utils/room_refs.py"],
    "u_generic": ["extractor/pyRevit/room_m/utils/generic.py"],
    "u_ui": ["extractor/pyRevit/room_m/utils/ui.py"],
    # ---- server
    "lib": ["src/lib.rs"],
    "main": ["src/main.rs"],
    "mcp": ["src/bin/mcp.rs"],
    "bootstrap": ["src/bootstrap.rs"],
    "handlers": ["src/handlers.rs"],
    "settings_api": ["src/settings_api.rs"],
    "svc_mod": ["src/service/mod.rs"],
    "svc_rooms": ["src/service/rooms.rs"],
    "svc_openings": ["src/service/openings.rs"],
    "svc_items": ["src/service/items.rs"],
    "svc_spaces": ["src/service/spaces.rs"],
    "svc_scope": ["src/service/entity_scope.rs"],
    "svc_areas": ["src/service/areas.rs"],
    "svc_adj": ["src/service/adjacency.rs"],
    "svc_cmp": ["src/service/comparison.rs"],
    "svc_val": ["src/service/validation.rs"],
    "svc_locator": ["src/service/room_locator.rs"],
    "svc_snap": ["src/service/snapshots.rs"],
    "svc_proj": ["src/service/projects.rs"],
    "svc_mile": ["src/service/milestones.rs"],
    "svc_ref": ["src/service/reference.rs"],
    "contract": ["src/contract/mod.rs"],
    "contract_openings": ["src/contract/openings.rs"],
    "contract_doors": ["src/contract/doors.rs"],
    "contract_windows": ["src/contract/windows.rs"],
    "contract_items": ["src/contract/items.rs"],
    "contract_ffe": ["src/contract/ffe.rs"],
    "contract_spaces": ["src/contract/spaces.rs"],
    "state": ["src/state.rs"],
    "classify": ["src/classify.rs"],
    "reference": ["src/reference.rs"],
    "storage": ["src/storage/mod.rs"],
    "storage_fs": ["src/storage/fs.rs"],
    "storage_mem": ["src/storage/mem.rs"],
    "settings": ["src/settings/mod.rs"],
    "settings_load": ["src/settings/load.rs"],
    "settings_validate": ["src/settings/validate.rs"],
    # ---- browser
    "js_index": ["src-js/renderer/index.ts"],
    "js_seam": ["src-js/renderer/seam.ts"],
    "js_types": ["src-js/renderer/types.ts"],
    "js_geom": ["src-js/renderer/geometry.ts"],
    "js_app": ["src-js/renderer/appearance.ts"],
    "js_gl": ["src-js/renderer/gl/renderer.ts"],
    "js_lines": ["src-js/renderer/gl/lines.ts"],
    "js_fills": ["src-js/renderer/gl/fills.ts"],
    "js_labels": ["src-js/renderer/gl/labels.ts"],
    "js_door": ["src-js/renderer/gl/doorGlyph.ts"],
    "js_window": ["src-js/renderer/gl/windowGlyph.ts"],
    "js_item": ["src-js/renderer/gl/itemGlyph.ts"],
    "js_spatial": ["src-js/renderer/gl/spatial.ts"],
    "js_proj": ["src-js/renderer/gl/projection.ts"],
    "js_viewport": ["src-js/renderer/gl/viewport.ts"],
    "js_colour": ["src-js/renderer/gl/colour.ts"],
    "js_paint": ["src-js/renderer/svg/paint.ts"],
    "p_index": ["static/index.html"],
    "p_settings": ["static/settings.html"],
    "p_comparison": ["static/comparison.html"],
    "p_graph": ["static/graph.js"],
    "p_common": ["static/common.js"],
    "p_tokens": ["static/tokens.css"],
    "p_bundle": ["static/vendor/renderer.bundle.js"],
    # ---- workbench. Globbed, because these are the two cells that are groups:
    # a new fixture generator or a sixth entity's probe should widen its cell
    # rather than need an edit here.
    "weekly": ["scripts/weekly_review.py"],
    "check_areas": ["scripts/check_areas.py"],
    "gen": sorted(glob.glob("scripts/gen_*.py")),
    "probes": sorted(glob.glob("scripts/probe_*.py")) + sorted(glob.glob("scripts/analyse_*.py")),
    "docsdir": sorted(glob.glob("docs/STRATEGY*.md")) + ["docs/CODING-CONVENTIONS.md"],
}

# A renderer module's tests live in a sibling `.test.ts` rather than inline, so
# they are folded into the module's own tests column. Without this each one
# would either vanish from the page or stand as a cell of its own, and neither
# matches how the Rust side is counted — which is the point of one definition.
SIBLING_TESTS = {
    "js_app": "src-js/renderer/appearance.test.ts",
    "js_geom": "src-js/renderer/geometry.test.ts",
    "js_colour": "src-js/renderer/gl/colour.test.ts",
    "js_door": "src-js/renderer/gl/doorGlyph.test.ts",
    "js_window": "src-js/renderer/gl/windowGlyph.test.ts",
    "js_item": "src-js/renderer/gl/itemGlyph.test.ts",
    "js_spatial": "src-js/renderer/gl/spatial.test.ts",
    "js_viewport": "src-js/renderer/gl/viewport.test.ts",
    "js_paint": "src-js/renderer/svg/paint.test.ts",
}


def measure_rust(text):
    """Rust, with `#[cfg(test)]` modules attributed to tests rather than code.

    The test module is found by brace matching from the attribute, not by a
    regex over the whole file: `mod tests` appears in prose and in strings, and
    the only thing that reliably ends the module is its own closing brace.

    **The matcher counts braces inside string literals, and that is a known
    hole with a deliberate fallback.** `storage/fs.rs` writes a raw byte string
    containing `{` — `br#"{"schema_version":7,...`— so its depth never returns
    to zero and the scan reaches the end of the file. Rather than guess, an
    unterminated module is taken as running **to the last line**, which is
    right here because this codebase puts the test module last in every file
    that has one. A file that ever grows code *after* its tests would be
    mismeasured, and the symptom is loud: the module's code count collapses
    toward zero. Teaching this to skip string literals is a real tokenizer, and
    the numbers are a cell width — not worth one until that layout changes.
    """
    lines = text.split("\n")
    in_tests = set()
    i = 0
    while i < len(lines):
        if re.match(r"\s*#\[cfg\(test\)\]", lines[i]):
            j = i
            while j < len(lines) and "{" not in lines[j]:
                j += 1
            depth, opened, k = 0, False, j
            while k < len(lines):
                depth += lines[k].count("{") - lines[k].count("}")
                opened = opened or "{" in lines[k]
                if opened and depth <= 0:
                    break
                k += 1
            # `k == len(lines)` is the unterminated case above: clamp to the
            # last real line rather than counting an index that does not exist.
            in_tests.update(range(i, min(k, len(lines) - 1) + 1))
            i = k + 1
            continue
        i += 1
    code, comments = _count_slash_comments(lines, skip=in_tests)
    return [code, len(lines), comments, len(in_tests)]


def _count_slash_comments(lines, skip=frozenset()):
    """`//` and `/* */` line classification, shared by Rust and TypeScript."""
    code = comments = 0
    in_block = False
    for n, line in enumerate(lines):
        if n in skip:
            continue
        s = line.strip()
        if not s:
            continue
        if in_block:
            comments += 1
            in_block = "*/" not in s
        elif s.startswith("/*"):
            comments += 1
            in_block = "*/" not in s
        elif s.startswith("//"):
            comments += 1
        else:
            code += 1
    return code, comments


def measure_python(text):
    lines = text.split("\n")
    code = comments = 0
    doc_quote = None
    for line in lines:
        s = line.strip()
        if not s:
            continue
        if doc_quote:
            comments += 1
            if doc_quote in s:
                doc_quote = None
        elif s[:3] in ('"""', "'''"):
            comments += 1
            doc_quote = None if s[:3] in s[3:] else s[:3]
        elif s.startswith("#"):
            comments += 1
        else:
            code += 1
    return [code, len(lines), comments, 0]


def measure_typescript(text, path):
    lines = text.split("\n")
    code, comments = _count_slash_comments(lines)
    if path.endswith(".test.ts"):
        return [0, len(lines), comments, code]
    return [code, len(lines), comments, 0]


def measure(path):
    """One file's `[code, whole file, comments, tests]`.

    HTML and CSS have no comment convention worth separating here — the viewer
    pages are markup, script and style in one document, and splitting them
    would be inventing a distinction the page does not draw.
    """
    with open(path, encoding="utf-8", errors="replace") as fh:
        text = fh.read()
    if path.endswith(".rs"):
        return measure_rust(text)
    if path.endswith(".py"):
        return measure_python(text)
    if path.endswith((".ts", ".js")):
        return measure_typescript(text, path)
    lines = text.split("\n")
    return [sum(1 for line in lines if line.strip()), len(lines), 0, 0]


def measure_all():
    """Every module id's totals, plus whatever could not be measured."""
    stats, missing = {}, []
    for module_id in sorted(MODULES):
        paths = MODULES[module_id]
        if not paths:
            missing.append((module_id, "no files matched"))
            continue
        totals = [0, 0, 0, 0]
        for path in paths:
            if not os.path.exists(path):
                missing.append((module_id, path))
                continue
            for idx, value in enumerate(measure(path)):
                totals[idx] += value
        sibling = SIBLING_TESTS.get(module_id)
        if sibling and os.path.exists(sibling):
            _, total, comments, tests = measure(sibling)
            totals[1] += total
            totals[2] += comments
            totals[3] += tests
        stats[module_id] = totals
    return stats, missing


def render_block(stats):
    rows = ",\n".join("    %s: [%d, %d, %d, %d]" % (k, *v) for k, v in stats.items())
    return "  const STATS = {\n%s\n  };" % rows


def committed_stats():
    """The `STATS` the plan currently draws, plus the ids its `M` array names.

    Both are read from the page rather than from a sidecar file, because the
    page is what a reader sees — a check against anything else would pass while
    the plan was wrong.
    """
    with open(PLAN, encoding="utf-8") as fh:
        html = fh.read()
    block = re.search(r"  const STATS = \{(.*?)\n  \};", html, re.S)
    if not block:
        sys.exit("could not find the STATS block in %s" % PLAN)
    stats = {
        k: [int(n) for n in v.split(",")]
        for k, v in re.findall(r"^    (\w+): \[([^\]]+)\]", block.group(1), re.M)
    }
    # `path:` is what distinguishes an `M` entry from a `LAYERS` or `ZONES` one,
    # which also open with an id and a name and are not modules.
    return stats, set(re.findall(r'id: "(\w+)", name: "[^"]*", path: "', html))


def check():
    """Report drift between the plan and the tree. Exit code is the answer."""
    measured, missing = measure_all()
    committed, drawn = committed_stats()
    problems = []
    for module_id, path in missing:
        problems.append("  %-20s measures a path that is gone: %s" % (module_id, path))
    for module_id in sorted(set(committed) - set(measured)):
        problems.append("  %-20s has a STATS row and no entry in MODULES" % module_id)
    for module_id in sorted(set(measured) - set(committed)):
        problems.append("  %-20s is in MODULES and has no STATS row" % module_id)
    for module_id in sorted(set(measured) & set(committed)):
        if measured[module_id] != committed[module_id]:
            problems.append(
                "  %-20s %s -> %s" % (module_id, committed[module_id], measured[module_id])
            )
    # The plan's own consistency: a cell with no numbers draws at no width, and
    # numbers with no cell are invisible. Neither shows up as a wrong figure.
    for module_id in sorted(drawn - set(committed)):
        problems.append("  %-20s is drawn by M with no STATS row" % module_id)

    if not problems:
        print("module plan STATS agree with the tree -- %d modules." % len(measured))
        return 0
    print("module plan STATS have drifted (%d):\n" % len(problems))
    print("\n".join(problems))
    print("\nRegenerate with: python scripts/module_stats.py")
    print("Blurbs and import edges are hand-written -- update those by hand.")
    return 1


if __name__ == "__main__":
    if "--check" in sys.argv[1:]:
        sys.exit(check())
    stats, missing = measure_all()
    for module_id, path in missing:
        print("warning: %s measures a path that is gone: %s" % (module_id, path), file=sys.stderr)
    print(render_block(stats))

#!/usr/bin/env python3
"""Mechanical drift checks for the weekly review — docs versus code.

**Why this exists.** This repo's documentation is its main asset: dense,
specific, and full of falsifiable claims about symbols, routes and file sizes.
That is a strength with a maintenance cost nobody can pay by re-reading. A
2026-08-07 review found ~20 drift findings, and all but a handful were
mechanically detectable — a doc naming `ServiceError::NotFound` eighteen months
after the variant was renamed is invisible to a careful reader and obvious to
`grep`. This script is that `grep`, with the false positives tuned out.

**This is advisory, never a gate, and that is a design decision rather than
timidity.** Live docs legitimately name symbols that do not exist, in at least
three shapes this script cannot distinguish from rot:

  - explaining a rename ("`ReferenceRecord`, renamed from `DrofusRecord`"),
  - recording a design that was considered and rejected,
  - describing a design settled but deliberately not built (STRATEGY-AUTHORED).

Each of those is *good writing*. A gate would punish it, and the pressure would
be to delete the history rather than fix the rot. So: this reports, a human
judges, and anything judged fine goes in `weekly_review_ignore.toml` **with a
reason** so next week is quieter than this week. The ignore file converging on
zero unexplained hits is the point.

`docs/Superseded/` is not checked at all. An archived handover naming a
since-renamed symbol is *correct history* — it records what was true when the
work landed. Checking it would generate permanent noise that no fix could
silence, which is the fastest way to get a tool like this ignored.

No third-party dependencies: stdlib only, so it runs anywhere the repo does.
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IGNORE_FILE = Path(__file__).resolve().parent / "weekly_review_ignore.toml"

# Where a symbol may live. Docs cite Rust types, TypeScript renderer internals,
# viewer globals and extractor functions interchangeably, so a check that read
# only `src/` would report every frontend and pyRevit symbol as dead.
SOURCE_TREES = ("src", "src-js", "static", "extractor", "scripts")
SOURCE_SUFFIXES = {".rs", ".ts", ".js", ".py", ".html"}

# Generated, and megabytes of minified vendor code: it would match almost any
# identifier by chance and silence real findings.
EXCLUDED_FILES = {"static/vendor/renderer.bundle.js"}


class Findings:
    """Collects results so every check can run before anything is printed."""

    def __init__(self) -> None:
        self.sections: list[tuple[str, list[str], str]] = []
        self.total = 0

    def add(self, title: str, hits: list[str], clean_note: str = "") -> None:
        self.sections.append((title, hits, clean_note))
        self.total += len(hits)

    def report(self) -> int:
        for title, hits, clean_note in self.sections:
            print(f"\n## {title}")
            if not hits:
                print(f"  OK{(' -- ' + clean_note) if clean_note else ''}")
                continue
            for h in hits:
                print(f"  {h}")
        print(f"\n{'=' * 68}")
        print(f"{self.total} item(s) to judge. None of this is automatically a defect --")
        print("see the header of scripts/weekly_review.py. Anything fine goes in")
        print("scripts/weekly_review_ignore.toml with a reason.")
        return 0


def load_ignores() -> dict:
    if not IGNORE_FILE.exists():
        return {}
    with IGNORE_FILE.open("rb") as fh:
        return tomllib.load(fh)


def live_docs() -> list[Path]:
    """Live strategy docs only — never `Superseded/`, never a review write-up.

    A review document deliberately quotes dead symbols as its subject matter, so
    checking one would report every finding it makes as a finding of its own.
    """
    return sorted(
        p
        for p in (ROOT / "docs").glob("*.md")
        if not p.name.startswith("CODE-REVIEW")
    )


def source_text() -> tuple[set[str], set[str]]:
    """Every identifier and every source path across all source trees."""
    words: set[str] = set()
    files: set[str] = set()
    for tree in SOURCE_TREES:
        for path in (ROOT / tree).rglob("*"):
            if path.suffix not in SOURCE_SUFFIXES or not path.is_file():
                continue
            rel = path.relative_to(ROOT).as_posix()
            files.add(rel)
            if rel in EXCLUDED_FILES:
                continue
            try:
                words.update(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", path.read_text(encoding="utf-8")))
            except UnicodeDecodeError:
                continue
    return words, files


# --------------------------------------------------------------------------
# Check 1 — symbol liveness
# --------------------------------------------------------------------------

RUST_PATH = re.compile(r"[a-z_]+(?:::[a-zA-Z_][a-zA-Z0-9_]*)+")
TYPE_NAME = re.compile(r"[A-Z][A-Za-z0-9]+")
FN_CALL = re.compile(r"[a-z_][a-z0-9_]*\(\)")
SRC_FILE = re.compile(r"(?:[a-z_][a-z0-9_]*/)*[a-z_][a-z0-9_]*\.(?:rs|ts|py)")


def check_symbols(findings: Findings, words: set[str], files: set[str], ignores: dict) -> None:
    """Docs name a symbol or source file that exists nowhere in the tree.

    An ignore key is either a bare symbol (benign wherever it appears -- an
    external API, a prose word) or `SYMBOL@DOC.md`, meaning benign *in that
    document only*. The scoped form matters: a doc explaining a rename
    ("the former `contract.rs`") is benign there, while the same name in a doc
    that means it as a live reference is still rot. A bare key would silence
    both, and the second is exactly what this check exists to find.
    """
    skip = set(ignores.get("symbols", {}).keys())
    hits: dict[str, list[str]] = {}

    def ignored(span: str, doc_name: str) -> bool:
        return span in skip or f"{span}@{doc_name}" in skip

    for doc in live_docs():
        for lineno, line in enumerate(doc.read_text(encoding="utf-8").splitlines(), 1):
            for span in re.findall(r"`([^`\n]{2,60})`", line):
                if ignored(span, doc.name):
                    continue
                if SRC_FILE.fullmatch(span):
                    if not any(f == span or f.endswith("/" + span) for f in files):
                        hits.setdefault(span, []).append(f"{doc.name}:{lineno}")
                elif RUST_PATH.fullmatch(span) or TYPE_NAME.fullmatch(span) or FN_CALL.fullmatch(span):
                    leaf = span.replace("()", "").split("::")[-1]
                    if leaf not in words:
                        hits.setdefault(span, []).append(f"{doc.name}:{lineno}")

    # Every site, never a truncated list: capping the locations at four silently
    # hid a real `contract.rs` reference in STRATEGY.md behind three benign ones,
    # which is the one failure mode a drift checker must not have.
    findings.add(
        "Symbols named in live docs that exist nowhere in the tree",
        [f"{sym:<38} {', '.join(locs)}" for sym, locs in sorted(hits.items())],
        f"{len(skip)} known-benign entr(y/ies) ignored",
    )


# --------------------------------------------------------------------------
# Check 2 — path liveness
# --------------------------------------------------------------------------

# A markdown link, or a bare citation that contains a directory separator.
#
# **Bare names are deliberately not checked**, and this is the single most
# important tuning decision in this file. Both the docs and the code cite
# documents by name constantly in prose -- "see PLAN-phasing.md D6",
# "(HANDOVER-gzip.md)" -- and those are navigational hints, not links. Checking
# them reported 60+ hits, nearly all of them prose that reads perfectly well and
# that no reader has ever been misled by.
#
# A citation that spells out a *path*, on the other hand, makes a checkable
# promise: it says "the file is here". Those are the ones that mislead, because
# they look authoritative and resolve to nothing. Every real finding the
# 2026-08-07 review made in this category (`docs/PLAN-webgl-renderer.md` cited
# from six build files after the doc moved to `Superseded/`) is of that shape.
MD_LINK = re.compile(r"\]\(\s*(?!https?:)([^)\s#]+\.md)(?:#[^)\s]*)?\s*\)")
MD_PATH = re.compile(r"(?<![\w/.])((?:\.\./|\./)*(?:[A-Za-z0-9_\-]+/)+[A-Za-z0-9_\-]+\.md)")


def check_paths(findings: Findings) -> None:
    """A `.md` *path* cited from code or live docs that no longer resolves.

    Catches the commonest rot in this repo by count: a document moves into
    `Superseded/` and the places spelling out its old path keep pointing at
    nothing. Nothing breaks -- a reader just lands nowhere, which is why it
    survives so long.
    """
    all_md = {p.relative_to(ROOT).as_posix() for p in ROOT.rglob("*.md") if "node_modules" not in p.parts}
    hits: list[str] = []

    scan: list[Path] = [ROOT / "package.json", ROOT / "tsconfig.json", ROOT / "vite.config.ts",
                        ROOT / ".gitignore", ROOT / "Cargo.toml", ROOT / "CLAUDE.md", ROOT / "README.md"]
    for base in [ROOT / t for t in SOURCE_TREES] + [ROOT / "docs", ROOT / "settings"]:
        if base.exists():
            scan.extend(p for p in base.rglob("*") if p.is_file() and p.suffix in SOURCE_SUFFIXES | {".md", ".toml"})

    seen: set[tuple[str, str]] = set()
    for path in scan:
        rel = path.relative_to(ROOT).as_posix()
        if not path.exists() or rel in EXCLUDED_FILES:
            continue
        # Archives cite their own era's paths correctly; this script's own files
        # and any review write-up quote broken paths as their subject matter.
        if "Superseded" in path.parts or path.name.startswith("CODE-REVIEW") or "weekly_review" in path.name:
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        for ref in set(MD_LINK.findall(text)) | set(MD_PATH.findall(text)):
            if (path.parent / ref).exists() or (ROOT / ref).exists():
                continue
            name = Path(ref).name
            actual = sorted(p for p in all_md if Path(p).name == name)
            if not actual:
                continue  # names a document that does not exist under any path
            key = (rel, ref)
            if key in seen:
                continue
            seen.add(key)
            hits.append(f"{rel:<38} cites {ref:<38} -> {actual[0]}")

    findings.add("Stale `.md` paths cited from code or live docs", sorted(hits))


# --------------------------------------------------------------------------
# Check 3 — HTTP route / MCP tool parity
# --------------------------------------------------------------------------

def check_mcp_parity(findings: Findings) -> None:
    """Every HTTP *read* route should have an MCP tool, per `bin/mcp.rs`'s claim.

    Mutating routes are deliberately absent from MCP (ingest, activate, settings
    writes) and are filtered out here rather than reported every week.
    """
    main = (ROOT / "src/main.rs").read_text(encoding="utf-8")
    mcp = (ROOT / "src/bin/mcp.rs").read_text(encoding="utf-8")

    routes: list[str] = []
    for m in re.finditer(r'\.route\(\s*"([^"]+)"\s*,\s*(.+?)\)\s*(?:\.layer|\n)', main, re.S):
        path, handlers = m.group(1), m.group(2)
        if "get(" in handlers:  # a read route; `post`-only routes are mutations
            routes.append(path)
    # `/comparison` is a POST that reads (it carries a list body), so it is a
    # read route despite the verb -- see handlers::compare_project_milestones.
    if "/projects/{id}/comparison" in main:
        routes.append("/projects/{id}/comparison")

    tool_count = len(re.findall(r"#\[tool\(", mcp))
    claimed = re.search(r"(\w+) in total", mcp)

    hits: list[str] = []
    # Route -> tool is judged by hand once, then encoded here: the mapping is
    # not derivable from names (`/areas` -> `get_hierarchy_areas`).
    mapping = {
        "/rooms": "get_rooms", "/doors": "get_doors", "/projects": "list_projects",
        "/projects/{id}/buildings": "list_buildings", "/projects/{id}/validation": "get_validation",
        "/projects/{id}/snapshots": "list_snapshots", "/projects/{id}/milestones": "list_milestones",
        "/projects/{id}/areas": "get_hierarchy_areas", "/projects/{id}/adjacency": "get_adjacency",
        "/projects/{id}/comparison": "compare_milestones",
        "/projects/{project_id}/models/{model_id}/snapshots/latest": "get_latest_snapshot",
        "/projects/{project_id}/models/{model_id}/snapshots/pending": "get_pending_snapshot",
        "/projects/{id}/reference/{source}/snapshots": "list_reference_snapshots",
        "/projects/{id}/reference/{source}/latest": "get_reference_snapshot",
        "/api/settings/projects": "list_project_settings",
        "/api/settings/projects/{id}": "get_project_settings",
        "/api/settings/resolve/{id}": None,
    }
    for route in routes:
        tool = mapping.get(route, "?")
        if tool == "?":
            hits.append(f"{route:<58} NEW read route -- no tool mapping recorded")
        elif tool is None:
            hits.append(f"{route:<58} no MCP tool (known gap -- is it still intended?)")
        elif f"fn {tool}(" not in mcp:
            hits.append(f"{route:<58} maps to `{tool}`, which is gone from bin/mcp.rs")

    numbers = {"Fourteen": 14, "Fifteen": 15, "Sixteen": 16, "Seventeen": 17, "Eighteen": 18}
    if claimed:
        said = numbers.get(claimed.group(1).capitalize())
        if said and said != tool_count:
            hits.append(f"bin/mcp.rs header says {claimed.group(1)} tools; {tool_count} `#[tool(` found")

    findings.add(
        "HTTP read routes without an MCP tool",
        hits,
        f"{len(routes)} read routes, {tool_count} tools",
    )


# --------------------------------------------------------------------------
# Check 4 — measured line counts in CODING-CONVENTIONS.md
# --------------------------------------------------------------------------

def real_lines(path: Path) -> int:
    """Non-test lines: everything before the first `#[cfg(test)]`.

    The convention doc judges module size by non-test lines but does not state
    how it counted. This proxy is stated here so the comparison is honest about
    being approximate, which is why the tolerance below is generous.
    """
    text = path.read_text(encoding="utf-8").splitlines()
    for i, line in enumerate(text):
        if "#[cfg(test)]" in line:
            return i
    return len(text)


def check_measured_numbers(findings: Findings, tolerance: float = 0.10) -> None:
    doc = ROOT / "docs/CODING-CONVENTIONS.md"
    text = doc.read_text(encoding="utf-8")
    hits: list[str] = []

    for m in re.finditer(r"`([a-z_/]+\.rs)`\s*\((\d[\d,]*)\)", text):
        name, claimed = m.group(1), int(m.group(2).replace(",", ""))
        lineno = text[: m.start()].count("\n") + 1
        # Resolve by exact path under `src/` first. Falling back to a bare
        # basename match would resolve `settings/mod.rs` to whichever `mod.rs`
        # came first -- it silently compared against `contract/mod.rs` and
        # reported a -16% drift that did not exist.
        exact = ROOT / "src" / name
        if exact.is_file():
            target = exact
        else:
            same_name = [p for p in (ROOT / "src").rglob("*.rs") if p.name == Path(name).name]
            target = same_name[0] if len(same_name) == 1 else None
        if target is None:
            hits.append(f"CODING-CONVENTIONS.md:{lineno:<4} `{name}` ({claimed}) -- file no longer exists")
            continue
        actual = real_lines(target)
        if abs(actual - claimed) > claimed * tolerance:
            drift = (actual - claimed) / claimed * 100
            hits.append(f"CODING-CONVENTIONS.md:{lineno:<4} `{name}` says {claimed}, measures ~{actual} ({drift:+.0f}%)")

    findings.add(
        "Measured line counts that have drifted",
        hits,
        f"within +/-{tolerance:.0%}",
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--only", choices=["symbols", "paths", "mcp", "numbers"], help="run one check")
    args = ap.parse_args()

    ignores = load_ignores()
    words, files = source_text()
    findings = Findings()

    print("RoomMate weekly review -- mechanical checks")
    print("=" * 68)

    if args.only in (None, "symbols"):
        check_symbols(findings, words, files, ignores)
    if args.only in (None, "paths"):
        check_paths(findings)
    if args.only in (None, "mcp"):
        check_mcp_parity(findings)
    if args.only in (None, "numbers"):
        check_measured_numbers(findings)

    return findings.report()


if __name__ == "__main__":
    sys.exit(main())

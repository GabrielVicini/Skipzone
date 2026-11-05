#!/usr/bin/env python3
"""Check that the documentation connects to the code.

Every document under docs/ documents a crate, is a dated report, or is a dated planning note.
This script fails the build when that stops being true:

1. Every derivation (docs/derivations/**/*.md) is cited by at least one .rs file.
2. Every citation of a docs file in .rs code resolves. A citation is the path
   from the repository root, optionally followed on the same line by section
   marks or equation numbers:

       docs/derivations/engine/haselgrove.md §2
       docs/derivations/engine/magnetic-field.md §1-3
       docs/derivations/iono/chapman-grazing.md eqs. (3), (8)

   A section mark resolves to a numbered heading in that file. Headings carry
   their number as the first word, with no trailing dot:

       ## 2 The Hamiltonian
       ### 2.1 Scaling

   An equation number resolves to a KaTeX tag, `\\tag{3}` (or `\\tag{3a}`,
   `\\tag{3b}`, ... for `(3)`). A section mark without a path on the same line,
   and the older "file.md section N" or "derivation eq. N" forms, are errors,
   because nothing could check them.
3. Each derivation starts with a front-matter list naming its crate, the code
   that implements it and the tests that check it. Every named file must
   exist and every named item must be defined in it. The "Section anchors
   used by code" line must list exactly the sections and equations the code
   cites.
4. Every markdown file under docs/ is linked from docs/README.md, and every
   relative link in docs/, the root README and the allowlisted READMEs points
   at a file that exists.
5. No markdown lives outside the places documentation is allowed to live.
   Anything else must be added to ALLOWED_MARKDOWN with a reason.

Standard library only. Run from anywhere: paths resolve against the
repository root (the parent of this script's directory).
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Markdown outside the documentation directories, each with the reason it
# stays. Paths are relative to the repository root.
ALLOWED_MARKDOWN = {
    "crates/skipzone-iono/data/nequick/README.md": (
        "documents the vendored NeQuick G data and carries the EUPL-1.2 "
        "notice that must travel with it"
    ),
}

# Prefixes (directories) and root-level names where markdown may live freely.
ALLOWED_PREFIXES = ("docs/", "wiki/", ".github/")
ALLOWED_ROOT_FILES = re.compile(r"^(README\.md|CHANGELOG\.md|LICENSE[^/]*)$")

# Directories never scanned for source or markdown.
SKIP_DIRS = {".git", "target", "node_modules"}

SECTION = r"\d+(?:\.\d+)*"
SECTION_ITEM = rf"§{SECTION}(?:-{SECTION})?"
EQ_ITEM = r"\([0-9]+[a-z]?\)"
CITATION = re.compile(
    r"(?P<path>docs/[A-Za-z0-9_./-]*[A-Za-z0-9_-]\.md)"
    rf"(?P<tail>(?:\s+{SECTION_ITEM}(?:,\s*{SECTION_ITEM})*)?"
    rf"(?:\s+eqs?\.\s*{EQ_ITEM}(?:,\s*{EQ_ITEM})*)?)"
)
LEGACY = [
    re.compile(r"\.md,?\s+(?:sections?|sec\.)\s*\d", re.IGNORECASE),
    re.compile(r"\bderivation\s+(?:sections?|sec\.|eqs?\.)\s*\d", re.IGNORECASE),
]
HEADING = re.compile(rf"^#{{2,6}}\s+(?P<num>{SECTION})\s+\S")
TAG = re.compile(r"\\tag\{(?P<tag>[0-9]+[a-z]?)\}")
FENCE = re.compile(r"^\s*(```|~~~)")
LINK = re.compile(r"\]\((?P<target>[^)\s]+)(?:\s+\"[^\"]*\")?\)")
FRONT_KEYS = (
    "Crate",
    "Implemented in",
    "Tested by",
    "Section anchors used by code",
)
DEFINITION = r"\b(?:fn|struct|enum|trait|const|static|type|mod|union)\s+{name}\b"


class Report:
    def __init__(self) -> None:
        self.errors: list[str] = []

    def error(self, where: str, msg: str) -> None:
        self.errors.append(f"{where}: {msg}")


def rel(p: Path) -> str:
    return p.relative_to(ROOT).as_posix()


def walk(suffix: str) -> list[Path]:
    """Files with this suffix, as git would see them (tracked or untracked,
    not ignored). Falls back to a filesystem walk outside a git checkout."""
    try:
        out = subprocess.run(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
            cwd=ROOT,
            capture_output=True,
            check=True,
        ).stdout.decode()
        files = [ROOT / f for f in out.split("\0") if f.endswith(suffix)]
        return sorted(f for f in files if f.is_file())
    except (OSError, subprocess.CalledProcessError):
        found = []
        for p in ROOT.rglob(f"*{suffix}"):
            if not SKIP_DIRS.intersection(p.relative_to(ROOT).parts) and p.is_file():
                found.append(p)
        return sorted(found)


def expand_sections(item: str) -> list[str]:
    """`§2` -> ['2'], `§1-3` -> ['1', '2', '3'], `§2.1-2.3` -> ['2.1', '2.2', '2.3']."""
    body = item.lstrip("§")
    if "-" not in body:
        return [body]
    lo, hi = body.split("-", 1)
    lo_parts, hi_parts = lo.split("."), hi.split(".")
    if len(lo_parts) != len(hi_parts) or lo_parts[:-1] != hi_parts[:-1]:
        return [lo, hi]
    prefix = ".".join(lo_parts[:-1])
    a, b = int(lo_parts[-1]), int(hi_parts[-1])
    if b < a:
        return [lo, hi]
    return [f"{prefix}.{i}" if prefix else str(i) for i in range(a, b + 1)]


class Doc:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.text = path.read_text(encoding="utf-8")
        self.sections: set[str] = set()
        self.tags: set[str] = set()
        in_fence = False
        for line in self.text.splitlines():
            if FENCE.match(line):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            m = HEADING.match(line)
            if m:
                self.sections.add(m.group("num"))
        self.tags = {m.group("tag") for m in TAG.finditer(self.text)}

    def has_equation(self, label: str) -> bool:
        if label in self.tags:
            return True
        return any(t[:-1] == label and t[-1].isalpha() for t in self.tags)

    def front_matter(self) -> dict[str, str]:
        found: dict[str, str] = {}
        for line in self.text.splitlines():
            m = re.match(r"^- \*\*(?P<key>[^*]+?):\*\*\s*(?P<value>.*)$", line)
            if m and m.group("key") in FRONT_KEYS:
                found[m.group("key")] = m.group("value")
            elif found and not line.startswith("- "):
                break
        return found


def check_code_citations(
    rep: Report, docs: dict[str, Doc], cited: dict[str, set[str]]
) -> None:
    for src in walk(".rs"):
        name = rel(src)
        for lineno, line in enumerate(src.read_text(encoding="utf-8").splitlines(), 1):
            where = f"{name}:{lineno}"
            for pat in LEGACY:
                if pat.search(line):
                    rep.error(
                        where,
                        "uncheckable citation form; write the docs path and a "
                        "section mark, e.g. docs/derivations/engine/haselgrove.md §2",
                    )
            spans = []
            for m in CITATION.finditer(line):
                spans.append(m.span())
                path = m.group("path")
                if not (ROOT / path).is_file():
                    rep.error(where, f"{path} does not exist")
                    continue
                doc = docs.get(path) or Doc(ROOT / path)
                docs.setdefault(path, doc)
                refs = cited.setdefault(path, set())
                refs.add("")  # cited at all
                tail = m.group("tail")
                for item in re.findall(SECTION_ITEM, tail):
                    for sec in expand_sections(item):
                        refs.add(f"§{sec}")
                        if sec not in doc.sections:
                            rep.error(
                                where,
                                f"{path} has no heading numbered {sec} "
                                f"(cited as {item}); headings look like '## {sec} Title'",
                            )
                for label in re.findall(r"\(([0-9]+[a-z]?)\)", tail):
                    refs.add(f"({label})")
                    if not doc.has_equation(label):
                        rep.error(where, f"{path} has no equation tagged \\tag{{{label}}}")
            for i, ch in enumerate(line):
                if ch == "§" and not any(a <= i < b for a, b in spans):
                    rep.error(
                        where,
                        "section mark without a docs path on the same line; "
                        "nothing can check what it refers to",
                    )


def resolve_item(rep: Report, where: str, crate: Path, item: str) -> None:
    file_part, _, path_part = item.partition("::")
    target = crate / file_part
    if not target.exists():
        rep.error(where, f"{rel(crate)}/{file_part} does not exist")
        return
    if not path_part:
        return
    if not target.is_file():
        rep.error(where, f"{item}: item paths need a file, not a directory")
        return
    text = target.read_text(encoding="utf-8")
    segments = path_part.split("::")
    last = segments[-1]
    if not re.search(DEFINITION.format(name=re.escape(last)), text):
        rep.error(where, f"{item}: no definition of `{last}` in {rel(target)}")
    for seg in segments[:-1]:
        if seg != "tests" and not re.search(rf"\b{re.escape(seg)}\b", text):
            rep.error(where, f"{item}: `{seg}` does not appear in {rel(target)}")


def check_is_test(rep: Report, where: str, crate: Path, item: str) -> None:
    file_part, _, path_part = item.partition("::")
    target = crate / file_part
    if not path_part or not target.is_file():
        rep.error(where, f"{item}: name a test as file::test_name")
        return
    last = path_part.rsplit("::", 1)[-1]
    text = target.read_text(encoding="utf-8")
    if not re.search(rf"#\[test\]\s*(?:#\[[^\]]*\]\s*)*fn\s+{re.escape(last)}\b", text):
        rep.error(where, f"{item}: `{last}` is not a #[test] function in {rel(target)}")


def check_front_matter(
    rep: Report, derivations: list[Doc], cited: dict[str, set[str]]
) -> None:
    for doc in derivations:
        name = rel(doc.path)
        fm = doc.front_matter()
        missing = [k for k in FRONT_KEYS if k not in fm]
        if missing:
            rep.error(
                name,
                "front matter is missing " + ", ".join(f"'- **{k}:**'" for k in missing)
                + " (a list directly under the title)",
            )
            continue
        crates = re.findall(r"`([^`]+)`", fm["Crate"])
        if not crates or not (ROOT / crates[0]).is_dir():
            rep.error(name, f"Crate {fm['Crate']!r} is not a directory")
            continue
        crate = ROOT / crates[0]
        for key in ("Implemented in", "Tested by"):
            items = re.findall(r"`([^`]+)`", fm[key])
            if not items:
                rep.error(name, f"'{key}' names nothing")
            for item in items:
                if key == "Tested by":
                    check_is_test(rep, f"{name} ({key})", crate, item)
                else:
                    resolve_item(rep, f"{name} ({key})", crate, item)
        listed: set[str] = set()
        for item in re.findall(SECTION_ITEM, fm["Section anchors used by code"]):
            listed.update(f"§{s}" for s in expand_sections(item))
        listed.update(
            f"({x})" for x in re.findall(r"\(([0-9]+[a-z]?)\)", fm["Section anchors used by code"])
        )
        used = cited.get(name, set()) - {""}
        if listed != used:
            extra = sorted(listed - used)
            lacking = sorted(used - listed)
            parts = []
            if lacking:
                parts.append("cited by code but not listed: " + ", ".join(lacking))
            if extra:
                parts.append("listed but not cited by code: " + ", ".join(extra))
            rep.error(name, "'Section anchors used by code' is out of date; " + "; ".join(parts))


def slugs(md: Path) -> set[str]:
    """GitHub's heading anchors for a markdown file: lower case, punctuation
    other than '-' and '_' dropped, spaces turned into hyphens, repeats
    numbered."""
    seen: dict[str, int] = {}
    out: set[str] = set()
    in_fence = False
    for line in md.read_text(encoding="utf-8").splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        m = re.match(r"^#{1,6}\s+(.*?)\s*#*\s*$", line)
        if in_fence or not m:
            continue
        slug = re.sub(r"[^\w\- ]", "", m.group(1).lower()).replace(" ", "-")
        n = seen.get(slug, 0)
        seen[slug] = n + 1
        out.add(slug if n == 0 else f"{slug}-{n}")
    return out


def check_links(rep: Report, files: list[Path]) -> None:
    for md in files:
        in_fence = False
        for lineno, line in enumerate(md.read_text(encoding="utf-8").splitlines(), 1):
            if FENCE.match(line):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            for m in LINK.finditer(line):
                target = m.group("target")
                if re.match(r"^[a-z][a-z0-9+.-]*:", target):
                    continue
                path, _, anchor = target.partition("#")
                dest = (md.parent / path).resolve() if path else md
                if not dest.exists():
                    rep.error(f"{rel(md)}:{lineno}", f"link target {path} does not exist")
                    continue
                if anchor and dest.suffix == ".md" and anchor not in slugs(dest):
                    rep.error(
                        f"{rel(md)}:{lineno}",
                        f"{rel(dest)} has no heading with anchor #{anchor}",
                    )


def check_index(rep: Report, docs_md: list[Path]) -> None:
    index = ROOT / "docs" / "README.md"
    if not index.is_file():
        rep.error("docs/README.md", "missing: docs/ needs a one-page index")
        return
    linked = set()
    for m in LINK.finditer(index.read_text(encoding="utf-8")):
        target = m.group("target").split("#", 1)[0]
        if target and not re.match(r"^[a-z][a-z0-9+.-]*:", target):
            linked.add((index.parent / target).resolve())
    for md in docs_md:
        if md != index and md.resolve() not in linked:
            rep.error(rel(md), "not linked from docs/README.md")


def check_orphans(rep: Report, all_md: list[Path]) -> None:
    for md in all_md:
        name = rel(md)
        if name.startswith(ALLOWED_PREFIXES) or ALLOWED_ROOT_FILES.match(name):
            continue
        if name in ALLOWED_MARKDOWN:
            continue
        rep.error(
            name,
            "markdown outside docs/, wiki/ and .github/; move it, delete it, or add "
            "it to ALLOWED_MARKDOWN in scripts/check-docs.py with a reason",
        )


def main() -> int:
    rep = Report()
    all_md = walk(".md")
    docs_md = [p for p in all_md if rel(p).startswith("docs/")]
    derivations = [Doc(p) for p in docs_md if rel(p).startswith("docs/derivations/")]
    docs = {rel(d.path): d for d in derivations}
    cited: dict[str, set[str]] = {}

    check_code_citations(rep, docs, cited)
    for d in derivations:
        if rel(d.path) not in cited:
            rep.error(rel(d.path), "not cited by any .rs file")
    check_front_matter(rep, derivations, cited)
    link_files = docs_md + [ROOT / "README.md"] + [ROOT / p for p in ALLOWED_MARKDOWN]
    check_links(rep, [p for p in link_files if p.is_file()])
    check_index(rep, docs_md)
    check_orphans(rep, all_md)

    if rep.errors:
        for e in rep.errors:
            print(f"error: {e}", file=sys.stderr)
        print(f"check-docs: {len(rep.errors)} error(s)", file=sys.stderr)
        return 1
    n_cites = sum(len(v) for v in cited.values())
    print(
        f"check-docs: ok ({len(derivations)} derivations, {len(cited)} cited files, "
        f"{n_cites} distinct citations, {len(all_md)} markdown files)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

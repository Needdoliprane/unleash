#!/usr/bin/env python3
"""Regenerate the per-file listing of .claude/INDEX.md (everything from "## Accueil" on).

Usage: python3 tools/design-package/index.py <package_root>
The header (title, summary paragraph, "Parcours conseillés") is kept as written.
Each file is listed under its top-level directory with the text of its first
Markdown heading (YAML front matter skipped); JSON/TOML/Python files have no title.
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import load_manifest, package_files  # noqa: E402

ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
C = ROOT / ".claude"
idx = C / "INDEX.md"
text = idx.read_text(encoding="utf-8")
head = text[: text.index("## Accueil")]


def title(rel):
    p = C / rel
    if p.suffix != ".md":
        return None
    t = p.read_text(encoding="utf-8")
    if t.startswith("---\n"):
        end = t.find("\n---", 4)
        t = t[end + 4:] if end != -1 else t
    m = re.search(r"^#\s+(.+?)\s*$", t, re.M)
    return m.group(1) if m else None


files = sorted(set(package_files(C, load_manifest(C))) | {"catalog/package-manifest.json"})
sections = {"": []}
for rel in files:
    top = rel.split("/", 1)[0] if "/" in rel else ""
    sections.setdefault(top, []).append(rel)
out = [head.rstrip("\n"), "", "## Accueil", ""]
for rel in sections.pop(""):
    t = title(rel)
    out.append(f"- [{rel}]({rel})" + (f" — {t}" if t else ""))
for top in sorted(sections):
    out += ["", f"## {top}", ""]
    for rel in sections[top]:
        t = title(rel)
        out.append(f"- [{rel}]({rel})" + (f" — {t}" if t else ""))
idx.write_text("\n".join(out) + "\n", encoding="utf-8")
print(f"INDEX.md: {len(files)} files listed")

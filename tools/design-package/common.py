"""Shared helpers for the OVPT Unleash design-package tools (standard library only)."""
import json
import os
from pathlib import Path

# Root files that belong to the design package export (presentation + agent bootstrap).
ROOT_FILES = ["AGENTS.md", "CLAUDE.md", "COMMENCER-ICI.html", "GEMINI.md", "LIRE-MOI.md", "SYNTHESE.md"]
# Root directories of agent bootstrap files covered by CHECKSUMS.sha256.
ROOT_DIRS = [".agents", ".codex"]
MEDIA = {
    ".md": "text/markdown",
    ".json": "application/json",
    ".toml": "application/toml",
    ".py": "text/x-python",
    ".html": "text/html",
}
IGNORED_NAMES = {".DS_Store", "__pycache__"}


def walk(base: Path, excluded_prefixes: tuple[str, ...] = ()):
    out = []
    for dp, dn, fn in os.walk(base):
        relative_dir = Path(dp).relative_to(base)
        dn[:] = [
            d for d in dn
            if d != ".git" and d not in IGNORED_NAMES
            and not ((relative_dir / d).as_posix() + "/").startswith(excluded_prefixes)
        ]
        for f in fn:
            if f in IGNORED_NAMES or f.endswith(".pyc"):
                continue
            # SMB can classify a directory symlink as a filename. Match os.walk's
            # default policy of leaving directory aliases outside the package.
            candidate = Path(dp) / f
            if candidate.is_symlink() and candidate.is_dir():
                continue
            relative_file = (relative_dir / f).as_posix()
            if not relative_file.startswith(excluded_prefixes):
                out.append(relative_file)
    return sorted(out)


def load_manifest(claude: Path):
    return json.loads((claude / "catalog/package-manifest.json").read_text(encoding="utf-8"))


def package_files(claude: Path, manifest=None):
    """Files of .claude/ that the manifest must list (relative to .claude/)."""
    m = manifest or load_manifest(claude)
    excl = set(m.get("excluded_paths", []))
    prefixes = tuple(m.get("excluded_prefixes", []))
    return [p for p in walk(claude, prefixes) if p not in excl]


def checksum_scope(root: Path, manifest=None):
    """Paths (relative to the package root) covered by CHECKSUMS.sha256."""
    claude = root / ".claude"
    m = manifest or load_manifest(claude)
    prefixes = tuple(m.get("excluded_prefixes", []))
    excluded_paths = set(m.get("excluded_paths", [])) - {"catalog/package-manifest.json"}
    scope = [".claude/" + p for p in walk(claude, prefixes) if p not in excluded_paths]
    for f in ROOT_FILES:
        if (root / f).is_file():
            scope.append(f)
    for d in ROOT_DIRS:
        if (root / d).is_dir():
            scope += [f"{d}/{p}" for p in walk(root / d)]
    return sorted(scope)

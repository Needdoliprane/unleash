#!/usr/bin/env python3
"""Regenerate .claude/catalog/package-manifest.json and CHECKSUMS.sha256.

Usage: python3 tools/design-package/regen.py <package_root>
The manifest lists every file of .claude/ except its own path, `excluded_paths`
and `excluded_prefixes` (development runtime state such as worktrees/ and state/).
`source_tree_digest` = SHA-256 of the compact UTF-8 JSON of the sorted
[{path, sha256, bytes}] list (ensure_ascii=False), as specified in
.claude/specs/document-package-ingestion.md.
"""
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import MEDIA, checksum_scope, load_manifest, package_files  # noqa: E402


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    claude = root / ".claude"
    mpath = claude / "catalog/package-manifest.json"
    man = load_manifest(claude)
    prefixes = man.setdefault("excluded_prefixes", [])
    if "evidence/" not in prefixes:
        prefixes.append("evidence/")
    entries = []
    for rel in package_files(claude, man):
        data = (claude / rel).read_bytes()
        ext = Path(rel).suffix
        if ext not in MEDIA:
            sys.exit(f"regen: unknown media type for .claude/{rel}")
        entries.append({"path": rel, "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data), "media_type": MEDIA[ext]})
    entries.sort(key=lambda e: e["path"])
    digest_input = [{"path": e["path"], "sha256": e["sha256"], "bytes": e["bytes"]} for e in entries]
    man["source_tree_digest"] = hashlib.sha256(
        json.dumps(digest_input, separators=(",", ":"), ensure_ascii=False).encode("utf-8")).hexdigest()
    man["files"] = entries
    mpath.write_text(json.dumps(man, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"manifest: {len(entries)} files, digest {man['source_tree_digest']}")

    lines = [f"{hashlib.sha256((root / p).read_bytes()).hexdigest()}  {p}" for p in checksum_scope(root, man)]
    (root / "CHECKSUMS.sha256").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"CHECKSUMS.sha256: {len(lines)} entries")


if __name__ == "__main__":
    main()

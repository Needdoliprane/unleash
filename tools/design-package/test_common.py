"""Vérifications du périmètre parcouru par les outils du dossier."""

from pathlib import Path
from unittest import TestCase
from unittest.mock import patch

import common


class PackageScopeTests(TestCase):
    ROOT = Path("/unleash-design-fixture")
    MANIFEST = {
        "excluded_prefixes": ["worktrees/", "state/", "catalog/temporary/"],
        "excluded_paths": ["catalog/package-manifest.json", "settings.local.json"],
    }

    def files(self, base):
        dirs = ["worktrees", "worktrees-old", "state", "catalog"]
        yield base, dirs, ["README.md", "settings.local.json"]
        self.assertEqual(dirs, ["worktrees-old", "catalog"])
        yield base / "worktrees-old", [], ["reference.md"]

        dirs = ["temporary"]
        yield base / "catalog", dirs, ["package-manifest.json", "operations.json"]
        self.assertEqual(dirs, [])

    def test_manifest_skips_excluded_directories_before_descending(self):
        with patch.object(common.os, "walk", self.files):
            self.assertEqual(
                common.package_files(self.ROOT / ".claude", self.MANIFEST),
                ["README.md", "catalog/operations.json", "worktrees-old/reference.md"],
            )

    def test_checksum_keeps_manifest_but_skips_excluded_directories(self):
        with (
            patch.object(common.os, "walk", self.files),
            patch.object(Path, "is_file", return_value=False),
            patch.object(Path, "is_dir", return_value=False),
        ):
            self.assertEqual(
                common.checksum_scope(self.ROOT, self.MANIFEST),
                [
                    ".claude/README.md",
                    ".claude/catalog/operations.json",
                    ".claude/catalog/package-manifest.json",
                    ".claude/worktrees-old/reference.md",
                ],
            )

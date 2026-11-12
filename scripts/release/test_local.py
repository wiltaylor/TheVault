import json
import tempfile
import unittest
from pathlib import Path

import local


class LocalReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.release = local.identity("a" * 40, "0.1.14-alpha.38", "release/0.1.14", "pre-release")

    def populate(self):
        for name in local.assets(self.release["version"]):
            (self.root / name).write_bytes(name.encode())

    def test_stable_identity(self):
        self.assertEqual(local.identity("b" * 40, "1.2.3", "release/1.2.3", "release")["kind"], "release")

    def test_identity_rejects_wrong_sha_version_branch_kind(self):
        for field, value in [("sha", "abc"), ("version", "0.1.14-alpha.99"),
                             ("branch", "next"), ("kind", "release")]:
            args = self.release | {field: value}
            with self.subTest(field=field), self.assertRaises(local.ReleaseError):
                local.identity(**args)

    def test_manifest_roundtrip(self):
        self.populate()
        recorded = local.manifest(self.root, self.release, True)
        local.verify_manifest(self.root, recorded, self.release)
        self.assertEqual(len(recorded["assets"]), 14)

    def test_manifest_rejects_changed_asset(self):
        self.populate()
        recorded = local.manifest(self.root, self.release, True)
        (self.root / "compose.yaml").write_text("changed")
        with self.assertRaises(local.ReleaseError):
            local.verify_manifest(self.root, recorded, self.release)

    def test_manifest_rejects_symlink(self):
        self.populate()
        (self.root / "compose.yaml").unlink()
        (self.root / "compose.yaml").symlink_to("Caddyfile")
        with self.assertRaises(local.ReleaseError):
            local.manifest(self.root, self.release, True)

    def test_manifest_rejects_missing_or_empty(self):
        self.populate()
        (self.root / "compose.yaml").write_bytes(b"")
        with self.assertRaises(local.ReleaseError):
            local.manifest(self.root, self.release, True)

    def test_unsigned_manifest_does_not_require_signature(self):
        self.populate()
        (self.root / f'TheVault_{self.release["version"]}_amd64.AppImage.sig').unlink()
        self.assertEqual(len(local.manifest(self.root, self.release, False)["assets"]), 13)

    def test_manifest_rejects_wrong_release(self):
        self.populate()
        recorded = local.manifest(self.root, self.release, True)
        with self.assertRaises(local.ReleaseError):
            local.verify_manifest(self.root, recorded, self.release | {"sha": "b" * 40})

    def test_lock_excludes_another_release(self):
        with local.release_lock(self.root):
            with self.assertRaises(local.ReleaseError):
                with local.release_lock(self.root):
                    self.fail("second release obtained lock")
        with local.release_lock(self.root):
            pass

    def test_resume_only_completed_prefix(self):
        path = self.root / "journal.json"
        journal = local.Journal(path, self.release)
        journal.complete("preflight", {"ok": True})
        resumed = local.Journal(path, self.release)
        self.assertEqual(resumed.data["completed"], ["preflight"])
        resumed.complete("cut", {"ok": True})
        self.assertEqual(json.loads(path.read_text())["completed"], ["preflight", "cut"])

    def test_no_state_before_success(self):
        path = self.root / "journal.json"
        local.Journal(path, self.release)
        self.assertFalse(path.exists())

    def test_refuse_resume_mismatched_sha(self):
        path = self.root / "journal.json"
        local.Journal(path, self.release).complete("preflight", {})
        with self.assertRaises(local.ReleaseError):
            local.Journal(path, self.release | {"sha": "b" * 40})

    def test_refuse_skipped_stage(self):
        with self.assertRaises(local.ReleaseError):
            local.Journal(self.root / "journal.json", self.release).complete("publish", {})

    def test_refuse_corrupt_stage_order(self):
        path = self.root / "journal.json"
        local.atomic_json(path, {"release": self.release, "completed": ["publish"]})
        with self.assertRaises(local.ReleaseError):
            local.Journal(path, self.release)


if __name__ == "__main__":
    unittest.main()

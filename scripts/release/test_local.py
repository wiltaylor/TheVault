import contextlib
import json
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

import local
import release as coordinator


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

    def config(self):
        return {"delivery": {"release_target": "test-release-client", "docker_release": {
            "destination": "test@relay", "port": 2251, "known_hosts": "test relay pin", "key": "must-not-leak"},
            "release_secrets": {"GH_TOKEN": "must-not-leak"}}}

    def test_delivery_identity_excludes_credentials(self):
        public = local.delivery_identity(self.config())
        self.assertNotIn("must-not-leak", json.dumps(public))

    def test_missing_delivery_route_fails_closed(self):
        for config in [{}, {"release_target": "client"}, {"release_target": "client", "docker_release": {"destination": "relay"}}]:
            with self.subTest(config=config), self.assertRaises(local.ReleaseError):
                local.delivery_identity(config)

    def test_dry_run_every_stage_has_target_identity_and_no_execution(self):
        plan = local.stage_plan(self.release, self.config())
        self.assertEqual([row["stage"] for row in plan], list(local.STAGES))
        for row in plan:
            self.assertTrue(row["dry_run"])
            self.assertEqual(row["sha"], self.release["sha"])
            self.assertEqual(row["version"], self.release["version"])
            self.assertTrue(row["target"])

    def job(self):
        with patch.dict("os.environ", {"DOCKER_HOST": "unix:///test-relay"}, clear=True):
            return coordinator.Release(self.root / "state", self.release, self.config(), self.root / "ram")

    def test_docker_environment_never_inherits_signing_or_publisher_credentials(self):
        with patch.dict("os.environ", {"GH_TOKEN": "secret", "TAURI_SIGNING_PRIVATE_KEY": "secret", "DOCKER_HOST": "unix:///relay"}):
            job = coordinator.Release(self.root / "state", self.release, self.config(), self.root / "ram")
        self.assertNotIn("secret", json.dumps(job.denv))
        self.assertNotIn("GH_TOKEN", job.base)

    def test_missing_keys_stops_before_signer_or_asset_mutation(self):
        job = self.job()
        with patch.dict("os.environ", {}, clear=True), patch.object(job, "run") as run:
            with self.assertRaisesRegex(local.ReleaseError, "waiting for operator signing prerequisites"):
                job.sign()
            run.assert_not_called()

    def test_cleanup_only_this_run_names(self):
        job = self.job()
        with patch.object(job, "docker") as docker:
            with self.assertRaises(local.ReleaseError):
                job.clean_container("someone-elses-container")
            docker.assert_not_called()
            job.clean_container(job.prefix + "-build")
            docker.assert_called_once_with("rm", "-f", "-v", job.prefix + "-build", check=False)

    def test_prepare_stops_before_smoke_and_publication(self):
        job = self.job()
        with contextlib.ExitStack() as stack:
            methods = {stage: stack.enter_context(patch.object(job, stage.replace("-", "_"), return_value={})) for stage in local.STAGES}
            self.assertIn("await signed smoke", job.execute())
            methods["smoke"].assert_not_called()
            methods["publish"].assert_not_called()
            self.assertEqual(job.journal.data["completed"], list(local.STAGES[:local.STAGES.index("smoke")]))

    def test_failure_does_not_advance_journal(self):
        job = self.job()
        with patch.object(job, "preflight", return_value={}), patch.object(job, "cut", side_effect=local.ReleaseError("red gate")):
            with self.assertRaisesRegex(local.ReleaseError, "red gate"):
                job.execute()
        self.assertEqual(job.journal.data["completed"], ["preflight"])

    def test_unsigned_smoke_evidence_rejected(self):
        job = self.job()
        evidence = self.root / "smoke.json"
        evidence.write_text(json.dumps({"sha": "b" * 40, "apk_sha256": "wrong", "status": "pass"}))
        with patch.object(job, "check_assets"):
            with self.assertRaisesRegex(local.ReleaseError, "does not match"):
                job.smoke(evidence, "TEST-1")

    def test_missing_smoke_run_id_rejected(self):
        job = self.job()
        with patch.object(job, "check_assets"):
            with self.assertRaisesRegex(local.ReleaseError, "waiting for signed APK smoke"):
                job.smoke(None, None)


if __name__ == "__main__":
    unittest.main()

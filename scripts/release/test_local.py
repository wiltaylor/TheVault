import contextlib
import json
import tempfile
import subprocess
import io
import tarfile
import base64
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

    def test_delivery_plan_does_not_read_credentials_or_relay_addresses(self):
        with patch.dict("os.environ", {"GH_TOKEN": "must-not-leak", "DOCKER_HOST": "unix:///private-job-socket"}):
            public = local.stage_plan(self.release)
        self.assertNotIn("must-not-leak", json.dumps(public))
        self.assertNotIn("private-job-socket", json.dumps(public))

    def test_missing_delivery_route_fails_closed(self):
        with patch.dict("os.environ", {}, clear=True), self.assertRaises(local.ReleaseError):
            coordinator.Release(self.root / "state", self.release, self.root / "ram")

    def test_dry_run_every_stage_has_target_identity_and_no_execution(self):
        plan = local.stage_plan(self.release)
        self.assertEqual([row["stage"] for row in plan], list(local.STAGES))
        for row in plan:
            self.assertTrue(row["dry_run"])
            self.assertEqual(row["sha"], self.release["sha"])
            self.assertEqual(row["version"], self.release["version"])
            self.assertTrue(row["target"])

    def job(self):
        with patch.dict("os.environ", {"DOCKER_HOST": "unix:///test-relay"}, clear=True):
            return coordinator.Release(self.root / "state", self.release, self.root / "ram")

    def test_docker_environment_never_inherits_signing_or_publisher_credentials(self):
        with patch.dict("os.environ", {"GH_TOKEN": "secret", "TAURI_SIGNING_PRIVATE_KEY": "secret", "DOCKER_HOST": "unix:///relay"}):
            job = coordinator.Release(self.root / "state", self.release, self.root / "ram")
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
            stack.enter_context(patch.object(job, "checkpoint"))
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

    def test_baseline_comparison_accepts_versioned_names_and_sizes(self):
        baseline = json.loads((Path(__file__).parent / "alpha37-assets.json").read_text())
        recorded = {"release": self.release, "assets": [dict(row, name=row["name"].replace(baseline["version"], self.release["version"])) for row in baseline["assets"]]}
        self.assertEqual(len(local.compare_baseline(recorded, baseline)), 14)
        recorded["assets"][0]["size"] = 1
        with self.assertRaisesRegex(local.ReleaseError, "asset size outside"):
            local.compare_baseline(recorded, baseline)

    def test_baseline_comparison_rejects_missing_asset(self):
        baseline = json.loads((Path(__file__).parent / "alpha37-assets.json").read_text())
        with self.assertRaisesRegex(local.ReleaseError, "asset inventory differs"):
            local.compare_baseline({"release": self.release, "assets": []}, baseline)

    def test_main_and_frozen_commit_tree_must_match(self):
        def fake_git(*args):
            if args == ("rev-parse", "HEAD"):
                return "b" * 40
            if args[0] == "rev-parse":
                return "tree-main" if args[1].startswith("b") else "tree-frozen"
            key = args[-1]
            if "key=pre-release," in key:
                return "true"
            if "key=tested-at-cut," in key:
                return "a" * 40 + " TEST-1"
            return ""
        with patch.object(coordinator, "git", side_effect=fake_git):
            with self.assertRaisesRegex(local.ReleaseError, "tree differs"):
                coordinator.freeze("0.1.14-alpha.38", None, resume=True)

    def test_record_frozen_sha_and_main_tag_sha_separately(self):
        def fake_git(*args):
            if args == ("rev-parse", "HEAD"):
                return "b" * 40
            if args[0] == "rev-parse":
                return "c" * 40
            key = args[-1]
            if "key=pre-release," in key:
                return "true"
            if "key=tested-at-cut," in key:
                return "a" * 40 + " TEST-1 TEST-2"
            return ""
        with patch.object(coordinator, "git", side_effect=fake_git):
            recorded = coordinator.freeze("0.1.14-alpha.38", None, resume=True)
        self.assertEqual(recorded["sha"], "a" * 40)
        self.assertEqual(recorded["tagged_sha"], "b" * 40)
        self.assertEqual(recorded["tree"], "c" * 40)

    def test_failed_custodial_command_does_not_print_output_or_environment(self):
        failed = subprocess.CompletedProcess(["signer"], 1, b"secret-output", b"private-key")
        with patch.object(coordinator.subprocess, "run", return_value=failed):
            with self.assertRaises(local.ReleaseError) as caught:
                coordinator.command(["signer"], env={"PASSWORD": "secret"})
        self.assertEqual(str(caught.exception), "signer failed (exit 1)")

    def test_source_fingerprint_detects_changed_contents_and_modes(self):
        source = self.root / "source"
        source.mkdir()
        path = source / "file"
        path.write_text("original")
        first = coordinator.source_digest(source)
        path.chmod(0o700)
        second = coordinator.source_digest(source)
        self.assertNotEqual(first, second)
        path.write_text("changed")
        self.assertNotEqual(second, coordinator.source_digest(source))

    def test_mocked_unsigned_build_uses_owned_container_and_no_host_bind(self):
        job = self.job()
        job.release["tagged_sha"] = "b" * 40
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w") as archive:
            item = tarfile.TarInfo("docker/dev.Dockerfile")
            item.size = 12
            archive.addfile(item, io.BytesIO(b"FROM scratch"))
        success = subprocess.CompletedProcess([], 0, stream.getvalue(), b"")
        def populate_outputs(*args, **kwargs):
            job.smoke_apk().parent.mkdir(parents=True)
            job.smoke_apk().write_bytes(b"unsigned-smoke")
            (job.out / "device").mkdir()
            (job.out / "device/smoke-fixtures.tar.gz").write_bytes(b"fixtures")
        with patch.object(job, "run", return_value=success), patch.object(job, "docker") as docker, patch.object(job, "container", side_effect=populate_outputs) as container:
            result = job.build()
        self.assertTrue(result["dev_image"].startswith(job.prefix))
        self.assertEqual(result["source_sha256"], coordinator.source_digest(job.source))
        self.assertIn("--workdir", container.call_args.kwargs["extra"])
        self.assertNotIn("--volume", container.call_args.kwargs["extra"])
        self.assertEqual(docker.call_args.args[0], "build")

    def signing_job(self):
        job = self.job()
        job.runtime.mkdir()
        job.dist.mkdir()
        for name in local.assets(job.release["version"]):
            (job.dist / name).write_bytes(name.encode())
        job.smoke_apk().parent.mkdir()
        job.smoke_apk().write_bytes(b"unsigned-smoke")
        job.journal.data["outputs"]["build"] = {"smoke_sha256": local.digest(job.smoke_apk())}
        job.journal.data["outputs"]["unsigned-manifest"] = job.unsigned_manifest()
        return job

    def test_mocked_signing_aligns_then_signs_checks_versions_and_commits(self):
        job = self.signing_job()
        env = {name: "dummy-test-value" for name in coordinator.SIGN_ENV}
        env["ANDROID_KEYSTORE_BASE64"] = base64.b64encode(b"dummy-keystore").decode()
        env["ANDROID_KEY_ALIAS"] = "test-key-alias"
        calls = []
        def fake_command(argv, **kwargs):
            calls.append(argv)
            stdout = b""
            if argv[0] == "zipalign":
                Path(argv[-1]).write_bytes(b"aligned")
            elif argv[:2] == ["apksigner", "sign"]:
                Path(argv[argv.index("--out") + 1]).write_bytes(b"signed-apk")
                self.assertIn("env:ANDROID_KEYSTORE_PASSWORD", argv)
            elif argv[:2] == ["apksigner", "verify"]:
                stdout = ("Signer #1 certificate SHA-256 digest: " + "ab" * 32).encode()
            elif argv[0] == "aapt2":
                stdout = b"package: versionCode='101438' versionName='0.1.14-alpha.38'"
            elif argv[0] == "bash":
                stdout = b"101438\n"
            elif argv[0] == "cargo":
                Path(argv[-1] + ".sig").write_bytes(base64.b64encode(b"trusted comment: app-version:0.1.14-alpha.38\n"))
            return subprocess.CompletedProcess(argv, 0, stdout, b"")
        with patch.dict("os.environ", env, clear=True), patch.object(coordinator, "android_signer", return_value="ab" * 32), patch.object(coordinator, "command", side_effect=fake_command):
            result = job.sign()
        operations = [argv[0] + (" " + argv[1] if argv[0] == "apksigner" else "") for argv in calls]
        self.assertLess(operations.index("zipalign"), operations.index("apksigner sign"))
        self.assertIn("minisign", operations)
        self.assertEqual(job.smoke_apk().read_bytes(), b"signed-apk")
        self.assertEqual(result["smoke_sha256"], local.digest(job.smoke_apk()))
        self.assertFalse((job.runtime / "android.jks").exists())
        self.assertNotIn("dummy-test-value", json.dumps(calls))

    def test_mocked_failed_signing_keeps_unsigned_inputs_and_removes_key(self):
        job = self.signing_job()
        env = {name: "dummy" for name in coordinator.SIGN_ENV}
        env.update(ANDROID_KEYSTORE_BASE64=base64.b64encode(b"dummy").decode())
        with patch.dict("os.environ", env, clear=True), patch.object(job, "run", side_effect=local.ReleaseError("zipalign failed")):
            with self.assertRaises(local.ReleaseError):
                job.sign()
        self.assertEqual(job.smoke_apk().read_bytes(), b"unsigned-smoke")
        self.assertFalse((job.runtime / "android.jks").exists())

    def test_mocked_phase_two_runs_smoke_before_publish_and_never_rebuilds(self):
        job = self.job()
        job.journal.data["completed"] = list(local.STAGES[:local.STAGES.index("smoke")])
        calls = []
        with contextlib.ExitStack() as stack:
            stack.enter_context(patch.object(job, "check_assets"))
            stack.enter_context(patch.object(job, "cleanup_images"))
            stack.enter_context(patch.object(job, "checkpoint"))
            for stage in local.STAGES:
                stack.enter_context(patch.object(job, stage.replace("-", "_"), side_effect=lambda *a, stage=stage: calls.append(stage) or {}))
            job.execute(True, self.root / "evidence", "TEST-1")
        self.assertEqual(calls, ["smoke", "publish", "verify"])

    def test_mocked_checkpoint_survives_fresh_client_without_secrets(self):
        job = self.job()
        job.release["tagged_sha"] = "b" * 40
        job.journal.data["completed"] = ["preflight", "cut"]
        local.atomic_json(job.out / "journal.json", job.journal.data)
        (job.out / "never-upload-key.jks").write_bytes(b"private-sentinel")
        uploaded = {}
        def fake_gh(*args, **kwargs):
            if args[:2] == ("release", "upload"):
                path = Path(args[-1])
                uploaded[path.name] = path.read_bytes()
                data = b""
            elif args[:2] == ("release", "download"):
                name = args[args.index("--pattern") + 1]
                directory = Path(args[args.index("--dir") + 1])
                (directory / name).write_bytes(uploaded[name])
                data = b""
            else:
                rows = [{"name": name, "size": len(data), "digest": "sha256:" + coordinator.hashlib.sha256(data).hexdigest()} for name, data in uploaded.items()]
                data = json.dumps({"draft": True, "target_commitish": "b" * 40, "assets": rows}).encode()
            return subprocess.CompletedProcess(args, 0, data, b"")
        with patch.object(job, "gh", side_effect=fake_gh):
            job.checkpoint()
        self.assertEqual(len(uploaded), 1)
        with tarfile.open(fileobj=io.BytesIO(next(iter(uploaded.values())))) as archive:
            self.assertNotIn("never-upload-key.jks", archive.getnames())
        fresh = self.root / "fresh-client"
        with patch.object(coordinator, "github", side_effect=lambda runtime, *a, **kw: fake_gh(*a, **kw)):
            restored = coordinator.restore_remote(fresh, job.release["version"], self.root / "ram")
        self.assertEqual(restored, job.release)
        self.assertEqual(json.loads((fresh / job.release["version"] / "journal.json").read_text())["completed"], ["preflight", "cut"])

    def test_failed_checkpoint_upload_does_not_delete_previous_checkpoint(self):
        job = self.job()
        job.journal.data["completed"] = ["preflight", "cut"]
        local.atomic_json(job.out / "journal.json", job.journal.data)
        with patch.object(job, "gh", side_effect=local.ReleaseError("upload interrupted")) as gh:
            with self.assertRaisesRegex(local.ReleaseError, "upload interrupted"):
                job.checkpoint()
        self.assertEqual(len(gh.call_args_list), 1)
        self.assertEqual(gh.call_args.args[:2], ("release", "upload"))


if __name__ == "__main__":
    unittest.main()

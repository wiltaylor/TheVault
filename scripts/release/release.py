#!/usr/bin/env python3
"""Two-phase local release. The daemon supplies DOCKER_HOST, never an SSH key.

prepare builds unsigned artifacts, signs at the custodian, tests images, uploads
a draft and pushes version-only image tags. finish requires signed smoke evidence
before publishing and advancing mutable tags. plan performs no external actions.
"""
from __future__ import annotations

import argparse
import base64
import contextlib
import io
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid
import urllib.error
import urllib.request

from local import (Journal, ReleaseError, STAGES, assets, atomic_json, compare_baseline,
                   digest, identity, manifest, release_lock,
                   stage_plan, verify_manifest, safe_extract)
from preflight import check_tools

ROOT = Path(__file__).resolve().parents[2]
REPO = "wiltaylor/TheVault"
REGISTRY = "ghcr.io/wiltaylor/"
BASE_ENV = ("PATH", "LANG", "LC_ALL", "TZ", "RUSTUP_HOME", "CARGO_HOME")
SIGN_ENV = ("ANDROID_KEYSTORE_BASE64", "ANDROID_KEYSTORE_PASSWORD", "ANDROID_KEY_ALIAS",
            "ANDROID_KEY_PASSWORD", "TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD")
CHECKPOINT = re.compile(r"thevault-release-checkpoint-([0-9]{2})-([0-9a-f]{64})\.tar\.gz")
SIGN_INPUT = re.compile(r"thevault-sign-input-([0-9a-f]{32})\.tar\.gz")


def android_signer():
    expected = (ROOT / "scripts/release/android-signer.sha256").read_text().strip()
    if not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ReleaseError("invalid committed Android certificate fingerprint")
    return expected


def check_token_scopes(name, token, opener=urllib.request.urlopen):
    if not token:
        raise ReleaseError("missing " + name + " from release-github")
    request = urllib.request.Request("https://api.github.com/user", headers={
        "Authorization": "Bearer " + token, "Accept": "application/vnd.github+json"})
    try:
        with opener(request, timeout=30) as response:
            scopes = {value.strip() for value in response.headers.get("X-OAuth-Scopes", "").split(",") if value.strip()}
    except (urllib.error.URLError, OSError):
        raise ReleaseError(name + " classic token validation failed") from None
    if not {"repo", "write:packages"}.issubset(scopes):
        raise ReleaseError(name + " must be a classic token with repo and write:packages scopes")
    return sorted(scopes)


def command(argv, *, cwd=ROOT, env=None, input=None, check=True):
    if env is None:
        env = {key: os.environ[key] for key in BASE_ENV if key in os.environ}
    result = subprocess.run(argv, cwd=cwd, env=env, input=input, capture_output=True)
    if check and result.returncode:
        # Child output may contain credentials. Never echo it from custodial tools.
        raise ReleaseError(f"{Path(argv[0]).name} failed (exit {result.returncode})")
    return result


def git(*args):
    return command(["git", *args], env={k: os.environ[k] for k in BASE_ENV if k in os.environ}).stdout.decode().strip()


def source_digest(source):
    rows = []
    for path in sorted(source.rglob("*")):
        if path.is_symlink():
            rows.append((str(path.relative_to(source)), "link", os.readlink(path)))
        elif path.is_file():
            rows.append((str(path.relative_to(source)), path.stat().st_mode & 0o777, digest(path)))
    return hashlib.sha256(json.dumps(rows).encode()).hexdigest()


def client_env(runtime):
    env = {key: os.environ[key] for key in BASE_ENV if key in os.environ}
    if os.environ.get("ANDROID_HOME"):
        env["PATH"] = str(Path(os.environ["ANDROID_HOME"]) / "build-tools/36.0.0") + os.pathsep + env.get("PATH", "")
    return env | {"HOME": str(runtime), "XDG_CONFIG_HOME": str(runtime),
                  "GH_CONFIG_DIR": str(runtime / "gh"), "DOCKER_CONFIG": str(runtime / "docker")}


def github(runtime, *args, check=True):
    if not os.environ.get("GH_TOKEN"):
        raise ReleaseError("missing GH_TOKEN from release-github")
    return command(["gh", *args], env=client_env(runtime) | {
        "GH_TOKEN": os.environ["GH_TOKEN"], "GH_PROMPT_DISABLED": "1"}, check=check)


def restore_remote(state, version, runtime):
    """Restore only hash-verified files from this version's authenticated draft."""
    # Validate before using a version in a path or tag.
    identity("0" * 40, version, "release/" + version.split("-")[0], "pre-release" if "-" in version else "release")
    tag = "v" + version
    result = github(runtime, "api", f"repos/{REPO}/releases/tags/{tag}", check=False)
    if result.returncode:
        if b"404" in result.stderr:
            return None
        raise ReleaseError("cannot inspect remote release recovery state")
    remote = json.loads(result.stdout)
    items = {item["name"]: item for item in remote["assets"]}
    out = state / version
    out.mkdir(parents=True, exist_ok=True)

    def download(name, directory):
        item = items[name]
        directory.mkdir(parents=True, exist_ok=True)
        github(runtime, "release", "download", tag, "--repo", REPO, "--pattern", name, "--dir", str(directory), "--clobber")
        path = directory / name
        if item.get("digest") != "sha256:" + digest(path) or item["size"] != path.stat().st_size:
            raise ReleaseError("remote recovery asset hash mismatch: " + name)
        return path

    checkpoints = [(int(CHECKPOINT.fullmatch(name)[1]), item.get("created_at", ""), name)
                   for name, item in items.items() if CHECKPOINT.fullmatch(name)]
    if checkpoints and remote["draft"]:
        name = max(checkpoints)[2]
        archive = download(name, state / "downloads")
        if digest(archive) != CHECKPOINT.fullmatch(name)[2]:
            raise ReleaseError("checkpoint filename hash mismatch")
        with tarfile.open(archive) as tar:
            safe_extract(tar, out)
        archive.unlink()
        journal = json.loads((out / "journal.json").read_text())
    elif "release-state.json" in items:
        capsule = json.loads(download("release-state.json", out).read_text())
        journal = capsule["journal"]
        atomic_json(out / "journal.json", journal)
        atomic_json(out / "run.json", capsule["run"])
        release = journal["release"]
        for name in assets(version):
            download(name, out / "assets")
        download("release-manifest.json", out)
        download(f"TheVault_{version}_x86_64.apk", out / "smoke")
        download("smoke-fixtures.tar.gz", out / "device")
    else:
        # An interrupted create/checkpoint upload can leave an empty draft.
        return None
    release = journal["release"]
    if release["version"] != version or release["tagged_sha"] != remote["target_commitish"]:
        raise ReleaseError("recovery state targets another release")
    Journal(out / "journal.json", release)  # validate stage prefix before use
    if "signed-smoke.json" in items:
        download("signed-smoke.json", out)
    return release


def freeze(version: str | None, branch: str | None, *, resume=False) -> dict:
    head = git("rev-parse", "HEAD")
    trailer = lambda key: git("log", "-1", f"--format=%(trailers:key={key},valueonly)")
    pre, stable = trailer("pre-release"), trailer("release")
    if (pre == "true") == (stable == "true"):
        raise ReleaseError("HEAD must carry exactly one release: true or pre-release: true trailer")
    gate = trailer("tested-at-cut").split()
    if len(gate) < 2 or not re.fullmatch(r"[a-f0-9]{40}", gate[0]):
        raise ReleaseError("tested-at-cut must name the frozen SHA and original test run IDs")
    frozen = gate[0]
    tree = git("rev-parse", head + "^{tree}")
    if tree != git("rev-parse", frozen + "^{tree}"):
        raise ReleaseError("tagged main commit tree differs from tested frozen release tree")
    kind = "pre-release" if pre == "true" else "release"
    wanted = trailer("version")
    if version and wanted and version.split("-")[0] != wanted:
        raise ReleaseError("selected version differs from the version trailer")
    if not resume or not version:
        # The job may be cloned from a tagless bundle. Refuse numbering from zero.
        if not git("tag", "--list", "v*"):
            command(["git", "fetch", "--no-recurse-submodules", f"https://github.com/{REPO}.git", "refs/tags/v*:refs/tags/v*"])
        if not git("tag", "--list", "v*"):
            raise ReleaseError("cannot calculate version without release tag history")
        calculated = command(["bash", "scripts/release/version.sh", kind, wanted]).stdout.decode().strip()
        if version and version != calculated:
            raise ReleaseError("requested version differs from trailer/tag calculation")
        version = calculated
    release = identity(frozen, version, branch or "release/" + version.split("-")[0], kind)
    release.update(tagged_sha=head, tree=tree, cut_runs=gate[1:])
    return release


@contextlib.contextmanager
def private_runtime():
    # Registry config and transient keystore material must never reach disk.
    mounts = Path("/proc/mounts").read_text().splitlines()
    if not any(line.split()[1:3] == ["/dev/shm", "tmpfs"] for line in mounts):
        raise ReleaseError("/dev/shm must be tmpfs for custodial tool state")
    with tempfile.TemporaryDirectory(prefix="thevault-release-", dir="/dev/shm") as directory:
        os.chmod(directory, 0o700)
        yield Path(directory)


class Release:
    def __init__(self, state: Path, release: dict, runtime: Path):
        self.state, self.release, self.runtime = state, release, runtime
        self.out = state / release["version"]
        self.out.mkdir(parents=True, exist_ok=True)
        self.journal = Journal(self.out / "journal.json", release)
        run_file = self.out / "run.json"
        if not run_file.exists():
            atomic_json(run_file, {"id": uuid.uuid4().hex[:12]})
        run_id = json.loads(run_file.read_text())["id"]
        if not re.fullmatch(r"[0-9a-f]{12}", run_id):
            raise ReleaseError("invalid checkpoint run identity")
        self.prefix = f'thevault-release-{release["version"]}-{run_id}'
        self.dist = self.out / "assets"
        self.source = self.out / "source"
        self.base = client_env(runtime)
        self.denv = self.base | {"DOCKER_HOST": os.environ.get("DOCKER_HOST") or os.environ.get("TERRARIUM_DOCKER_HOST", "")}
        if not self.denv["DOCKER_HOST"]:
            raise ReleaseError("missing daemon-provided Docker release relay")
        if not self.denv["DOCKER_HOST"].startswith("unix:///"):
            raise ReleaseError("release requires the daemon-provided Unix Docker relay, not a TCP port")

    def run(self, argv, **kwargs):
        return command(argv, env=self.base, **kwargs)

    def docker(self, *args, **kwargs):
        return command(["docker", *args], env=self.denv, **kwargs)

    def gh(self, *args, check=True):
        return github(self.runtime, *args, check=check)

    def clean_container(self, name):
        if not name.startswith(self.prefix + "-"):
            raise ReleaseError("refusing cleanup outside this release run")
        self.docker("rm", "-f", "-v", name, check=False)

    def container(self, suffix, image, argv, *, copies=(), outputs=(), extra=()):
        name = self.prefix + "-" + suffix
        self.clean_container(name)
        self.docker("create", "--name", name, *extra, image, *argv)
        try:
            for source, dest in copies:
                self.docker("cp", str(source), f"{name}:{dest}")
            result = self.docker("start", "--attach", name, check=False)
            for source, dest in outputs:
                Path(dest).mkdir(parents=True, exist_ok=True)
                self.docker("cp", f"{name}:{source}", str(dest))
            status = self.docker("inspect", "--format", "{{.State.ExitCode}}", name).stdout.strip()
            if result.returncode or status != b"0":
                # Build output is secret-free, so retain it as diagnostic evidence.
                (self.out / f"{suffix}.log").write_bytes(result.stdout + result.stderr)
                raise ReleaseError(f"{suffix} container failed, see {suffix}.log")
            (self.out / f"{suffix}.log").write_bytes(result.stdout + result.stderr)
            return result
        finally:
            self.clean_container(name)

    def preflight(self):
        self.docker("version")
        if git("diff", "--name-only", "HEAD"):
            raise ReleaseError("release checkout has tracked modifications")
        return {"relay": "available", "client_tools": getattr(self, "tool_versions", {})}

    def cut(self):
        if not self.release.get("cut_runs"):
            raise ReleaseError("missing tested-at-cut run IDs")
        self.ensure_draft()
        return {"frozen_sha": self.release["sha"], "tree": self.release["tree"], "runs": self.release["cut_runs"]}

    def build(self):
        # git archive excludes untracked state and secret files. Never copy HOME.
        archive = self.run(["git", "archive", self.release["tagged_sha"]]).stdout
        # A failed build can leave a stamped source copy. Recreate only this
        # run's owned source directory before extracting the immutable archive.
        if self.source.exists():
            shutil.rmtree(self.source)
        self.source.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            safe_extract(tar, self.source)
        dev = self.prefix + "-dev:build"
        self.docker("build", "-f", str(self.source / "docker/dev.Dockerfile"), "-t", dev, str(self.source / "docker"))
        # Root performs only directory ownership, then drops to numeric uid 1000.
        # No host bind mounts, Docker socket, SSH keys or publisher env enter it.
        self.container("unsigned", dev, ["bash", "-c",
            "mkdir -p /src /cargo /gradle /pnpm-store /tmp/release-home; "
            "getent passwd 1000 >/dev/null || useradd --uid 1000 --home-dir /tmp/release-home release-build; "
            "chown -R 1000:1000 /src /cargo /gradle /pnpm-store /tmp/release-home; "
            "exec setpriv --reuid=1000 --regid=1000 --clear-groups bash scripts/release/build-unsigned.sh \"$1\"", "_", self.release["version"]],
            copies=[(str(self.source) + "/.", "/src")], outputs=[("/src/out/.", self.out)],
            extra=["--workdir", "/src", "--env", "HOME=/tmp/release-home", "--env", "CARGO_BUILD_JOBS=8", "--env", "RUST_TEST_THREADS=8",
                   "--env", "THEVAULT_WEB_COMMIT=" + self.release["tagged_sha"][:7]])
        # Stamp only the owned source copy for the image builds, never job HEAD.
        self.run(["bash", str(self.source / "scripts/release/stamp.sh"), self.release["version"]], cwd=self.source)
        return {"dev_image": dev, "smoke_sha256": digest(self.smoke_apk()),
                "source_sha256": source_digest(self.source),
                "fixtures_sha256": digest(self.out / "device/smoke-fixtures.tar.gz")}

    def smoke_apk(self):
        return self.out / "smoke" / f'TheVault_{self.release["version"]}_x86_64.apk'

    def unsigned_manifest(self):
        value = manifest(self.dist, self.release, False)
        original = self.out / "unsigned"
        if original.exists():
            shutil.rmtree(original)
        shutil.copytree(self.dist, original / "assets")
        shutil.copy2(self.smoke_apk(), original / self.smoke_apk().name)
        atomic_json(self.out / "unsigned-manifest.json", value)
        return {"manifest_sha256": digest(self.out / "unsigned-manifest.json")}

    def sign(self):
        mode = os.environ.get("THEVAULT_SIGNING_MODE", "github")
        if mode == "github":
            return self.sign_github()
        if mode == "custodian":
            return self.sign_custodian()
        raise ReleaseError("THEVAULT_SIGNING_MODE must be github or custodian")

    def sign_github(self):
        from signing import filenames, input_manifest, verify_outputs
        version = self.release["version"]
        original = self.out / "unsigned"
        if digest(self.out / "unsigned-manifest.json") != self.journal.data["outputs"]["unsigned-manifest"]["manifest_sha256"]:
            raise ReleaseError("unsigned manifest changed before workflow signing")
        verify_manifest(original / "assets", json.loads((self.out / "unsigned-manifest.json").read_text()), self.release)
        unsigned = self.out / "github-inputs"
        unsigned.mkdir(exist_ok=True)
        for name in filenames(version):
            source = original / name if name.endswith("_x86_64.apk") else original / "assets" / name
            shutil.copy2(source, unsigned / name)
        if digest(unsigned / filenames(version)[1]) != self.journal.data["outputs"]["build"]["smoke_sha256"]:
            raise ReleaseError("unsigned x86_64 APK changed before workflow signing")
        inputs = input_manifest(unsigned, version, self.release["tagged_sha"])
        atomic_json(unsigned / "input-manifest.json", inputs)
        request_path = self.out / "sign-request.json"
        if request_path.exists():
            request = json.loads(request_path.read_text())
            if request["inputs"] != inputs or not re.fullmatch(r"[a-f0-9]{32}", request["id"]):
                raise ReleaseError("workflow signing request identity changed")
        else:
            request_id = uuid.uuid4().hex
            bundle = self.state / f"thevault-sign-input-{request_id}.tar.gz"
            with tarfile.open(bundle, "w:gz") as tar:
                for name in [*filenames(version), "input-manifest.json"]:
                    tar.add(unsigned / name, arcname=name)
            request = {"id": request_id, "inputs": inputs, "bundle_sha": digest(bundle)}
            self.gh("release", "upload", "v" + version, "--repo", REPO, str(bundle))
            atomic_json(request_path, request)
            self.checkpoint()  # survive reset between dispatch and artifact collection
            bundle.unlink()
        request_id = request["id"]

        def find_run():
            response = self.gh("run", "list", "--repo", REPO, "--workflow", "sign-only.yml", "--event", "workflow_dispatch",
                               "--limit", "100", "--json", "databaseId,displayTitle,status,conclusion")
            matches = [row for row in json.loads(response.stdout) if row["displayTitle"] == "Sign-only " + request_id]
            if len(matches) > 1:
                raise ReleaseError("ambiguous sign-only workflow request")
            return matches[0] if matches else None

        workflow = find_run()
        if not workflow:
            self.gh("workflow", "run", "sign-only.yml", "--repo", REPO, "--ref", "main",
                    "-f", "version=" + version, "-f", "source_sha=" + self.release["tagged_sha"],
                    "-f", "request_id=" + request_id, "-f", "bundle_asset=thevault-sign-input-" + request_id + ".tar.gz",
                    "-f", "bundle_sha=" + request["bundle_sha"])
            for _ in range(10):
                workflow = find_run()
                if workflow:
                    break
                time.sleep(2)
            if not workflow:
                raise ReleaseError("sign-only dispatch accepted; run not visible yet, resume this release")
        run_id = str(workflow["databaseId"])
        if request.get("run_id") not in (None, run_id):
            raise ReleaseError("workflow signing run changed for the saved request")
        if "run_id" not in request:
            request["run_id"] = run_id
            atomic_json(request_path, request)
            self.checkpoint()
        print(f"sign-only request {request_id}: workflow run {run_id}", flush=True)
        if workflow["status"] == "completed" and workflow["conclusion"] != "success":
            self.gh("run", "rerun", run_id, "--repo", REPO, "--failed")
        self.gh("run", "watch", run_id, "--repo", REPO, "--exit-status")
        signed = self.out / "github-signed"
        if signed.exists():
            shutil.rmtree(signed)
        signed.mkdir()
        self.gh("run", "download", run_id, "--repo", REPO, "--name", "thevault-signing-" + request_id, "--dir", str(signed))
        verify_outputs(unsigned, signed, version, self.release["tagged_sha"], runner=lambda argv: self.run(argv).stdout)
        for name in filenames(version) + [f"TheVault_{version}_amd64.AppImage.sig"]:
            destination = self.smoke_apk() if name.endswith("_x86_64.apk") else self.dist / name
            shutil.copy2(signed / name, destination)
        self.write_sums()
        return {"smoke_sha256": digest(self.smoke_apk()), "workflow_run": run_id, "request_id": request_id}

    def sign_custodian(self):
        missing = [name for name in SIGN_ENV if name != "ANDROID_KEY_PASSWORD" and not os.environ.get(name)]
        if missing:
            raise ReleaseError("waiting for operator signing prerequisites: " + ", ".join(missing))
        original = self.out / "unsigned"
        if digest(self.out / "unsigned-manifest.json") != self.journal.data["outputs"]["unsigned-manifest"]["manifest_sha256"]:
            raise ReleaseError("unsigned manifest changed")
        verify_manifest(original / "assets", json.loads((self.out / "unsigned-manifest.json").read_text()), self.release)
        # Recover a partial signing commit from immutable unsigned inputs.
        for name in assets(self.release["version"]):
            if (original / "assets" / name).exists():
                shutil.copy2(original / "assets" / name, self.dist / name)
        shutil.copy2(original / self.smoke_apk().name, self.smoke_apk())
        if digest(self.smoke_apk()) != self.journal.data["outputs"]["build"]["smoke_sha256"]:
            raise ReleaseError("unsigned smoke APK changed")
        key = self.runtime / "android.jks"
        key.write_bytes(base64.b64decode(os.environ["ANDROID_KEYSTORE_BASE64"], validate=True))
        key.chmod(0o600)
        env = self.base | {name: os.environ[name] for name in SIGN_ENV if name in os.environ}
        env.setdefault("ANDROID_KEY_PASSWORD", env["ANDROID_KEYSTORE_PASSWORD"])
        version = self.release["version"]
        signed = self.out / "signing"
        signed.mkdir(exist_ok=True)
        inputs = [self.dist / f"TheVault_{version}_aarch64.apk", self.smoke_apk()]
        try:
            for apk in inputs:
                aligned, output = signed / (apk.name + ".aligned"), signed / apk.name
                self.run(["zipalign", "-f", "-p", "4", str(apk), str(aligned)])
                command(["apksigner", "sign", "--v4-signing-enabled", "false", "--ks", str(key), "--ks-pass", "env:ANDROID_KEYSTORE_PASSWORD",
                         "--ks-key-alias", env["ANDROID_KEY_ALIAS"], "--key-pass", "env:ANDROID_KEY_PASSWORD",
                         "--out", str(output), str(aligned)], env=env)
                cert = self.run(["apksigner", "verify", "--min-sdk-version", "33", "--print-certs", str(output)]).stdout.decode()
                expected = android_signer()
                found = re.findall(r"certificate SHA-256 digest: ([0-9a-fA-F]+)", cert)
                if found != [expected]:
                    raise ReleaseError("APK signer differs from existing shipped certificate")
                badging = self.run(["aapt2", "dump", "badging", str(output)]).stdout.decode()
                code = self.run(["bash", "scripts/release/version-code.sh", version]).stdout.decode().strip()
                if f"versionCode='{code}' versionName='{version}'" not in badging:
                    raise ReleaseError("signed APK version mismatch")
            appimage = self.dist / f"TheVault_{version}_amd64.AppImage"
            command(["cargo-tauri", "signer", "sign", "--app-version", version, str(appimage)], env=env)
            pubkey = json.loads((ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text())["plugins"]["updater"]["pubkey"]
            public, signature = self.runtime / "updater.pub", self.runtime / "updater.sig"
            public.write_bytes(base64.b64decode(pubkey))
            signature.write_bytes(base64.b64decode(Path(str(appimage) + ".sig").read_bytes()))
            self.run(["minisign", "-Vm", str(appimage), "-p", str(public), "-x", str(signature)])
            if not re.search(r"(?<![\w.-])" + re.escape(version) + r"(?![\w.-])", signature.read_text().split("trusted comment:")[-1]):
                raise ReleaseError("AppImage signature does not bind the release version")
            # Commit APK replacement only after every signing verification passes.
            for apk in inputs:
                os.replace(signed / apk.name, apk)
            self.write_sums()
        finally:
            key.unlink(missing_ok=True)
        return {"smoke_sha256": digest(self.smoke_apk())}

    def write_sums(self):
        groups = {"SHA256SUMS": ("desktop", "updater-signature", "deb", "rpm", "aur"),
                  "SHA256SUMS-android": ("android",), "SHA256SUMS-cli": ("cli",)}
        for filename, roles in groups.items():
            rows = [f"{digest(self.dist / name)}  {name}\n" for name, role in assets(self.release["version"]).items() if role in roles]
            (self.dist / filename).write_text("".join(rows))

    def signed_manifest(self):
        value = manifest(self.dist, self.release, True)
        comparison = compare_baseline(value, json.loads((ROOT / "scripts/release/alpha37-assets.json").read_text()))
        atomic_json(self.out / "asset-comparison.json", {"comparisons": comparison})
        atomic_json(self.out / "release-manifest.json", value)
        return {"manifest_sha256": digest(self.out / "release-manifest.json"), "smoke_sha256": digest(self.smoke_apk())}

    def check_assets(self):
        recorded = json.loads((self.out / "release-manifest.json").read_text())
        verify_manifest(self.dist, recorded, self.release)
        expected = self.journal.data["outputs"]["signed-manifest"]
        if digest(self.out / "release-manifest.json") != expected["manifest_sha256"] or digest(self.smoke_apk()) != expected["smoke_sha256"]:
            raise ReleaseError("signed release manifest or smoke APK changed")
        if digest(self.out / "device/smoke-fixtures.tar.gz") != self.journal.data["outputs"]["build"]["fixtures_sha256"]:
            raise ReleaseError("signed smoke fixtures changed")
        return recorded

    def images(self):
        self.check_assets()
        if source_digest(self.source) != self.journal.data["outputs"]["build"]["source_sha256"]:
            raise ReleaseError("image source changed since unsigned builds")
        plain, server, browser = [self.prefix + f"-{name}:build" for name in ("plain", "server", "browser")]
        version, sha = self.release["version"], self.release["tagged_sha"]
        labels = ["--label", "org.opencontainers.image.source=https://github.com/" + REPO,
                  "--label", "org.opencontainers.image.version=" + version,
                  "--label", "org.opencontainers.image.revision=" + sha]
        server_args = ["build", "--platform", "linux/amd64", "--build-arg", "THEVAULT_WEB_COMMIT=" + sha[:7],
                       "-f", str(self.source / "docker/Dockerfile")]
        self.docker(*server_args, "-t", plain, str(self.source))
        filenames = [name for name, role in assets(version).items() if role in ("android", "desktop", "cli")]
        clients = self.out / "clients"
        self.container("clients", plain, ["-c", 'mkdir -p /clients; exec vault clients publish --dir /clients --build release "$@"', "_",
                       *["/assets/" + name for name in filenames]],
                       copies=[(self.dist, "/assets")], outputs=[("/clients/.", clients)],
                       extra=["--user", "0:0", "--entrypoint", "/bin/sh"])
        self.docker(*server_args, "--build-context", "clients=" + str(clients), *labels, "-t", server, str(self.source))
        name = self.prefix + "-server-test"
        self.clean_container(name)
        self.docker("run", "-d", "--name", name, "--env", "VAULT_PASSWORD=release-image-test-only", server)
        try:
            self.container("server-probe", self.journal.data["outputs"]["build"]["dev_image"],
                           ["python3", "/probe.py", version, "/manifest.json"],
                           copies=[(ROOT / "scripts/release/server-probe.py", "/probe.py"),
                                   (self.out / "release-manifest.json", "/manifest.json")],
                           extra=["--network", "container:" + name])
        finally:
            self.clean_container(name)
        self.docker("build", "--platform", "linux/amd64", "-f", str(self.source / "docker/browser.Dockerfile"),
                    *labels, "-t", browser, str(self.source))
        # This path uses docker exec for HTTP access, not the release client's localhost.
        result = command(["bash", "docker/test-browser-image.sh"], cwd=self.source,
                         env=self.denv | {"IMAGE": browser, "THEVAULT_RELEASE_RELAY": "1", "THEVAULT_RELEASE_RUN": self.prefix + "-browser-test"}, check=False)
        (self.out / "browser-test.log").write_bytes(result.stdout + result.stderr)
        if result.returncode:
            raise ReleaseError("browser image test failed, see browser-test.log")
        output = {}
        for name, tag in (("thevault", server), ("thevault-browser", browser)):
            archive = self.out / (name + ".tar")
            self.docker("save", "--output", str(archive), tag)
            output[name] = {"archive": archive.name, "sha256": digest(archive),
                            "local_tag": tag,
                            "manifest_digest": self.run(["crane", "digest", "--tarball", str(archive)]).stdout.decode().strip(),
                            "image_id": self.docker("image", "inspect", "--format", "{{.Id}}", tag).stdout.decode().strip()}
        return output

    def upload_files(self):
        return [*(self.dist / name for name in assets(self.release["version"])),
                self.out / "release-manifest.json", self.smoke_apk(), self.out / "device/smoke-fixtures.tar.gz"]

    def ensure_draft(self):
        tag = "v" + self.release["version"]
        # Network/auth errors are not evidence of absence. Only a 404 is absence.
        ref = self.gh("api", f"repos/{REPO}/git/ref/tags/{tag}", check=False)
        if ref.returncode == 0:
            raise ReleaseError("tag already exists before draft preparation")
        elif b"404" not in ref.stderr:
            raise ReleaseError("cannot establish release tag absence")
        existing = self.gh("api", f"repos/{REPO}/releases/tags/{tag}", check=False)
        if existing.returncode == 0:
            value = json.loads(existing.stdout)
            if not value["draft"] or value["target_commitish"] != self.release["tagged_sha"]:
                raise ReleaseError("existing release is published or targets another commit")
        else:
            if b"404" not in existing.stderr:
                raise ReleaseError("cannot establish draft absence")
            self.gh("release", "create", tag, "--repo", REPO, "--draft", "--target", self.release["tagged_sha"],
                    "--title", "TheVault " + tag, "--generate-notes", "--latest=false",
                    "--prerelease=" + str(self.release["kind"] == "pre-release").lower())
            value = {"assets": []}
        return value

    def draft(self):
        self.check_assets()
        tag = "v" + self.release["version"]
        value = self.ensure_draft()
        remote = {asset["name"]: asset for asset in value["assets"] if not CHECKPOINT.fullmatch(asset["name"]) and not SIGN_INPUT.fullmatch(asset["name"])}
        if set(remote) - {path.name for path in self.upload_files()}:
            raise ReleaseError("draft contains unexpected assets")
        for path in self.upload_files():
            if path.name in remote:
                item = remote[path.name]
                if item.get("digest") != "sha256:" + digest(path) or item["size"] != path.stat().st_size:
                    raise ReleaseError("draft asset mismatch: " + path.name)
            else:
                self.gh("release", "upload", tag, "--repo", REPO, str(path))
        return {"tag": tag, "target": self.release["tagged_sha"]}

    def registry_env(self):
        token = os.environ.get("GHCR_TOKEN")
        if not token:
            raise ReleaseError("missing GHCR_TOKEN from release-github")
        config = self.runtime / "registry"
        config.mkdir(mode=0o700, exist_ok=True)
        auth = base64.b64encode((os.environ.get("GHCR_USER", "wiltaylor") + ":" + token).encode()).decode()
        atomic_json(config / "config.json", {"auths": {"ghcr.io": {"auth": auth}}})
        return self.base | {"DOCKER_CONFIG": str(config)}

    def crane(self, *args, check=True):
        return command(["crane", *args], env=self.registry_env(), check=check)

    def push(self):
        self.check_assets()
        pushed = {}
        for name, image in self.journal.data["outputs"]["images"].items():
            if name not in ("thevault", "thevault-browser") or image["archive"] != name + ".tar":
                raise ReleaseError("invalid checkpoint image identity")
            archive = self.out / image["archive"]
            if not archive.exists():
                suffix = "server" if name == "thevault" else "browser"
                tag = self.prefix + "-" + suffix + ":build"
                if image["local_tag"] != tag or self.docker("image", "inspect", "--format", "{{.Id}}", tag).stdout.decode().strip() != image["image_id"]:
                    raise ReleaseError("tested relay image missing or changed; refusing rebuild on resume")
                self.docker("save", "--output", str(archive), tag)
            elif digest(archive) != image["sha256"]:
                raise ReleaseError("tested image archive changed")
            ref = REGISTRY + name + ":" + self.release["version"]
            want = self.crane("digest", "--tarball", str(archive)).stdout.decode().strip()
            if want != image["manifest_digest"]:
                raise ReleaseError("exported image differs from tested manifest digest")
            found = self.crane("digest", ref, check=False)
            if found.returncode == 0:
                if found.stdout.decode().strip() != want:
                    raise ReleaseError("immutable version image tag mismatch")
            else:
                # crane push uses a local tarball and local credentials. Neither
                # credential nor registry auth is sent to the Docker relay.
                if b"MANIFEST_UNKNOWN" not in found.stderr and b"NAME_UNKNOWN" not in found.stderr:
                    raise ReleaseError("cannot establish registry version tag absence")
                self.crane("push", str(archive), ref)
            if self.crane("digest", ref).stdout.decode().strip() != want:
                raise ReleaseError("registry digest differs from tested image")
            pushed[name] = {"ref": ref, "digest": want}
        return pushed

    def smoke(self, evidence: Path | None, run_id: str | None):
        self.check_assets()
        if not evidence or not run_id:
            raise ReleaseError("prepared draft and version images: waiting for signed APK smoke evidence and original run ID")
        report = json.loads(evidence.read_text())
        if report.get("status") != "pass" or report.get("sha") != self.release["sha"] or report.get("apk_sha256") != digest(self.smoke_apk()):
            raise ReleaseError("signed smoke evidence does not match the frozen SHA and signed APK")
        return {"run_id": run_id, "sha256": digest(evidence), "apk_sha256": report["apk_sha256"]}

    def publish(self):
        self.check_assets()
        tag = "v" + self.release["version"]
        # Recheck every uploaded byte before making the draft visible. A remote
        # draft edited between phases must not be published by this run.
        value = json.loads(self.gh("api", f"repos/{REPO}/releases/tags/{tag}").stdout)
        if value["target_commitish"] != self.release["tagged_sha"]:
            raise ReleaseError("draft target changed after prepare")
        by_name = {asset["name"]: asset for asset in value["assets"] if not CHECKPOINT.fullmatch(asset["name"]) and not SIGN_INPUT.fullmatch(asset["name"])}
        if set(by_name) != {path.name for path in self.upload_files()} | {"release-state.json", "signed-smoke.json"}:
            raise ReleaseError("draft asset inventory changed after prepare")
        for path in self.upload_files():
            if by_name.get(path.name, {}).get("digest") != "sha256:" + digest(path):
                raise ReleaseError("draft asset changed after prepare: " + path.name)
        if by_name["signed-smoke.json"].get("digest") != "sha256:" + self.journal.data["outputs"]["smoke"]["sha256"]:
            raise ReleaseError("remote signed smoke evidence changed")
        ref = self.gh("api", f"repos/{REPO}/git/ref/tags/{tag}", check=False)
        if ref.returncode == 0:
            if json.loads(ref.stdout)["object"]["sha"] != self.release["tagged_sha"]:
                raise ReleaseError("tag changed after prepare")
        elif b"404" not in ref.stderr:
            raise ReleaseError("cannot verify tag before publishing")
        # Save the verified finish handoff before removing private recovery
        # bundles. A client reset after this point resumes from public assets.
        self.save_capsule()
        for asset in value["assets"]:
            if CHECKPOINT.fullmatch(asset["name"]) or SIGN_INPUT.fullmatch(asset["name"]):
                self.gh("release", "delete-asset", tag, asset["name"], "--repo", REPO, "--yes")
        # Mutable tags only advance after the signed smoke stage has been recorded.
        for name, image in self.journal.data["outputs"]["push"].items():
            if self.crane("digest", image["ref"]).stdout.decode().strip() != image["digest"]:
                raise ReleaseError("version tag moved since prepare")
            channels = ["edge"]
            if self.release["kind"] == "release":
                channels += ["latest", ".".join(self.release["version"].split(".")[:2])]
            for channel in channels:
                self.crane("tag", REGISTRY + name + "@" + image["digest"], channel)
        self.gh("release", "edit", tag, "--repo", REPO, "--draft=false",
                "--prerelease=" + str(self.release["kind"] == "pre-release").lower(),
                "--latest=" + str(self.release["kind"] == "release").lower())
        return {"tag": tag}

    def verify(self):
        tag = "v" + self.release["version"]
        value = json.loads(self.gh("api", f"repos/{REPO}/releases/tags/{tag}").stdout)
        if value["draft"] or value["prerelease"] != (self.release["kind"] == "pre-release"):
            raise ReleaseError("GitHub release visibility/kind mismatch")
        ref = json.loads(self.gh("api", f"repos/{REPO}/git/ref/tags/{tag}").stdout)
        if ref["object"]["sha"] != self.release["tagged_sha"]:
            raise ReleaseError("published tag SHA mismatch")
        by_name = {a["name"]: a for a in value["assets"]}
        for path in self.upload_files():
            if by_name.get(path.name, {}).get("digest") != "sha256:" + digest(path):
                raise ReleaseError("published asset digest mismatch: " + path.name)
        for name, image in self.journal.data["outputs"]["push"].items():
            channels = [self.release["version"], "edge"]
            if self.release["kind"] == "release":
                channels += ["latest", ".".join(self.release["version"].split(".")[:2])]
            for channel in channels:
                if self.crane("digest", REGISTRY + name + ":" + channel).stdout.decode().strip() != image["digest"]:
                    raise ReleaseError("published image channel digest mismatch")
        return {"url": value["html_url"]}

    def cleanup_images(self):
        # Exact owned tags only. Base layers and other releases are untouched.
        for suffix in ("dev", "plain", "server", "browser"):
            self.docker("image", "rm", self.prefix + "-" + suffix + ":build", check=False)

    def save_capsule(self):
        capsule = self.out / "release-state.json"
        atomic_json(capsule, {"journal": self.journal.data, "run": json.loads((self.out / "run.json").read_text())})
        remote = json.loads(self.gh("api", f'repos/{REPO}/releases/tags/v{self.release["version"]}').stdout)
        if not remote["draft"]:
            existing = next((a for a in remote["assets"] if a["name"] == capsule.name), {})
            if existing.get("digest") != "sha256:" + digest(capsule):
                raise ReleaseError("refusing to change published release recovery metadata")
            return
        self.gh("release", "upload", "v" + self.release["version"], "--repo", REPO, "--clobber", str(capsule))

    def checkpoint(self):
        completed = self.journal.data["completed"]
        if "cut" not in completed or "publish" in completed:
            return
        if "push" in completed:
            self.save_capsule()
        archive = self.state / "checkpoint.tar.gz"
        # No credentials ever enter out. Explicit allowlist also excludes
        # image tarballs, runtime state, and transient signing intermediates.
        allowed = ("journal.json", "run.json", "source", "assets", "unsigned", "smoke", "device",
                   "unsigned-manifest.json", "release-manifest.json", "asset-comparison.json", "sign-request.json")
        with tarfile.open(archive, "w:gz") as tar:
            for name in allowed:
                path = self.out / name
                if path.exists():
                    tar.add(path, arcname=name)
        if archive.stat().st_size >= 2 * 1024**3:
            raise ReleaseError("private checkpoint exceeds GitHub's 2 GiB asset limit")
        name = f"thevault-release-checkpoint-{len(completed):02}-{digest(archive)}.tar.gz"
        named = self.state / name
        os.replace(archive, named)
        tag = "v" + self.release["version"]
        # New immutable checkpoint first, delete old owned checkpoints only
        # after its successful upload. An interrupted upload retains the old one.
        self.gh("release", "upload", tag, "--repo", REPO, str(named))
        value = json.loads(self.gh("api", f"repos/{REPO}/releases/tags/{tag}").stdout)
        if not any(a["name"] == name and a.get("digest") == "sha256:" + digest(named) for a in value["assets"]):
            raise ReleaseError("uploaded checkpoint digest mismatch")
        for asset in value["assets"]:
            if asset["name"] != name and CHECKPOINT.fullmatch(asset["name"]):
                self.gh("release", "delete-asset", tag, asset["name"], "--repo", REPO, "--yes")
        named.unlink()

    def execute(self, finish=False, evidence=None, run_id=None):
        if "signed-manifest" in self.journal.data["completed"]:
            self.check_assets()
        for stage in STAGES:
            if stage in self.journal.data["completed"]:
                continue
            if stage == "smoke" and not finish:
                return "prepared: draft and version-only images await signed smoke"
            print("release stage:", stage, flush=True)
            method = getattr(self, stage.replace("-", "_"))
            result = method(evidence, run_id) if stage == "smoke" else method()
            self.journal.complete(stage, result)
            self.checkpoint()
        self.cleanup_images()
        return "published and verified"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("publish", "plan"))
    parser.add_argument("--state", type=Path, default=Path("release-out"),
                        help="job-local scratch; private draft checkpoints survive client reset")
    parser.add_argument("--version", help="optional version assertion, or test version for plan")
    parser.add_argument("--branch")
    parser.add_argument("--plan-sha", help="plan only: test frozen SHA without release trailers")
    args = parser.parse_args(argv)

    def stop(signum, _frame):
        raise ReleaseError(f"interrupted by signal {signum}; resume the same release")
    for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        signal.signal(sig, stop)
    try:
        with release_lock(args.state):
            if args.mode == "plan":
                if args.plan_sha:
                    version = args.version or "0.1.14-alpha.38"
                    release = identity(args.plan_sha, version, args.branch or "release/" + version.split("-")[0],
                                       "pre-release" if "-" in version else "release")
                else:
                    release = freeze(args.version, args.branch)
                print(json.dumps({"stages": stage_plan(release), "assets": assets(release["version"])}, indent=2))
                return 0
            if args.plan_sha:
                raise ReleaseError("--plan-sha cannot authorize execution")
            phase = git("log", "-1", "--format=%(trailers:key=release-phase,valueonly)") or "prepare"
            if phase not in ("prepare", "finish"):
                raise ReleaseError("release-phase must be prepare or finish")
            state = args.state.resolve()
            with private_runtime() as runtime:
                versions = check_tools(lambda argv, **kw: command(argv, env=client_env(runtime), **kw), os.environ.get("ANDROID_HOME"))
                for name in ("GH_TOKEN", "GHCR_TOKEN"):
                    check_token_scopes(name, os.environ.get(name))
                run_id = None
                if phase == "finish":
                    version = git("log", "-1", "--format=%(trailers:key=release-version,valueonly)")
                    if not version or (args.version and args.version != version):
                        raise ReleaseError("finish needs the exact release-version trailer")
                    run_id = git("log", "-1", "--format=%(trailers:key=signed-smoke,valueonly)")
                    if not re.fullmatch(r"[A-Za-z0-9_-]+", run_id):
                        raise ReleaseError("finish needs signed-smoke: <original test run ID>")
                    release = restore_remote(state, version, runtime)
                    if not release:
                        raise ReleaseError("no prepared draft recovery state for finish")
                    if git("rev-parse", "HEAD^{tree}") != release["tree"]:
                        raise ReleaseError("finish commit tree differs from prepared release tree")
                    command(["git", "merge-base", "--is-ancestor", release["tagged_sha"], "HEAD"])
                else:
                    head = git("rev-parse", "HEAD")
                    response = github(runtime, "api", f"repos/{REPO}/releases?per_page=100")
                    releases = json.loads(response.stdout)
                    if any(not row["draft"] and row["target_commitish"] == head for row in releases):
                        raise ReleaseError("this main commit is already published; refusing a second version")
                    drafts = [row for row in releases if row["draft"]]
                    matching = [row for row in drafts if row["target_commitish"] == head]
                    if len(matching) > 1:
                        raise ReleaseError("multiple drafts target this commit")
                    if any(row["target_commitish"] != head for row in drafts):
                        raise ReleaseError("another draft release is unfinished; finish it before a new cut")
                    if matching:
                        version = matching[0]["tag_name"].removeprefix("v")
                        if args.version and args.version != version:
                            raise ReleaseError("requested version differs from existing draft")
                        release = freeze(version, args.branch, resume=True)
                        restored = restore_remote(state, version, runtime)
                        if restored and restored != release:
                            raise ReleaseError("remote checkpoint SHA/tree/version mismatch")
                    else:
                        release = freeze(args.version, args.branch)
                job = Release(state, release, runtime)
                job.tool_versions = versions
                evidence = job.out / "signed-smoke.json" if phase == "finish" else None
                if phase == "finish" and "smoke" in job.journal.data["completed"]:
                    if job.journal.data["outputs"]["smoke"]["run_id"] != run_id:
                        raise ReleaseError("finish smoke run ID differs from recorded handoff")
                print(job.execute(phase == "finish", evidence, run_id))
            return 0
    except (ReleaseError, ValueError, KeyError, OSError, tarfile.TarError) as error:
        print("release stopped:", str(error) if isinstance(error, ReleaseError) else type(error).__name__, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

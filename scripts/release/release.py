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
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import uuid

from local import (Journal, ReleaseError, STAGES, assets, atomic_json,
                   delivery_identity, digest, identity, manifest, release_lock,
                   stage_plan, verify_manifest)

ROOT = Path(__file__).resolve().parents[2]
REPO = "wiltaylor/TheVault"
REGISTRY = "ghcr.io/wiltaylor/"
BASE_ENV = ("PATH", "LANG", "LC_ALL", "TZ", "RUSTUP_HOME", "CARGO_HOME")
SIGN_ENV = ("ANDROID_KEYSTORE_BASE64", "ANDROID_KEYSTORE_PASSWORD", "ANDROID_KEY_ALIAS",
            "ANDROID_KEY_PASSWORD", "TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD")


def command(argv, *, cwd=ROOT, env=None, input=None, check=True):
    result = subprocess.run(argv, cwd=cwd, env=env, input=input, capture_output=True)
    if check and result.returncode:
        # Child output may contain credentials. Never echo it from custodial tools.
        raise ReleaseError(f"{Path(argv[0]).name} failed (exit {result.returncode})")
    return result


def git(*args):
    return command(["git", *args], env={k: os.environ[k] for k in BASE_ENV if k in os.environ}).stdout.decode().strip()


def freeze(version: str | None, branch: str | None) -> dict:
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
    if not version:
        # The job may be cloned from a tagless bundle. Refuse numbering from zero.
        if not git("tag", "--list", "v*"):
            command(["git", "fetch", "--no-recurse-submodules", f"https://github.com/{REPO}.git", "refs/tags/v*:refs/tags/v*"])
        if not git("tag", "--list", "v*"):
            raise ReleaseError("cannot calculate version without release tag history")
        version = command(["bash", "scripts/release/version.sh", kind, trailer("version")]).stdout.decode().strip()
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
    def __init__(self, state: Path, release: dict, config: dict, runtime: Path):
        self.state, self.release, self.runtime = state, release, runtime
        self.routing = delivery_identity(config)
        self.out = state / release["version"]
        self.out.mkdir(parents=True, exist_ok=True)
        self.journal = Journal(self.out / "journal.json", release | {"delivery": self.routing})
        run_file = self.out / "run.json"
        if not run_file.exists():
            atomic_json(run_file, {"id": uuid.uuid4().hex[:12]})
        self.prefix = f'thevault-release-{release["version"]}-{json.loads(run_file.read_text())["id"]}'
        self.dist = self.out / "assets"
        self.source = self.out / "source"
        self.base = {key: os.environ[key] for key in BASE_ENV if key in os.environ}
        self.base.update(HOME=str(runtime), XDG_CONFIG_HOME=str(runtime),
                         GH_CONFIG_DIR=str(runtime / "gh"), DOCKER_CONFIG=str(runtime / "docker"))
        self.denv = self.base | {"DOCKER_HOST": os.environ.get("DOCKER_HOST") or os.environ.get("TERRARIUM_DOCKER_HOST", "")}
        if not self.denv["DOCKER_HOST"]:
            raise ReleaseError("missing daemon-provided Docker release relay")

    def run(self, argv, **kwargs):
        return command(argv, env=self.base, **kwargs)

    def docker(self, *args, **kwargs):
        return command(["docker", *args], env=self.denv, **kwargs)

    def gh(self, *args, check=True):
        if not os.environ.get("GH_TOKEN"):
            raise ReleaseError("missing GH_TOKEN from release-github")
        return command(["gh", *args], env=self.base | {"GH_TOKEN": os.environ["GH_TOKEN"], "GH_PROMPT_DISABLED": "1"}, check=check)

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
        return {"relay": "available"}

    def cut(self):
        if not self.release.get("cut_runs"):
            raise ReleaseError("missing tested-at-cut run IDs")
        return {"frozen_sha": self.release["sha"], "tree": self.release["tree"], "runs": self.release["cut_runs"]}

    def build(self):
        # git archive excludes untracked state and secret files. Never copy HOME.
        archive = self.run(["git", "archive", self.release["tagged_sha"]]).stdout
        self.source.mkdir(exist_ok=True)
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(self.source, filter="data")
        dev = self.prefix + "-dev:build"
        self.docker("build", "-f", str(self.source / "docker/dev.Dockerfile"), "-t", dev, str(self.source / "docker"))
        # Root performs only directory ownership, then drops to numeric uid 1000.
        # No host bind mounts, Docker socket, SSH keys or publisher env enter it.
        self.container("unsigned", dev, ["bash", "-c",
            "mkdir -p /src /cargo /gradle /pnpm-store /tmp/release-home; "
            "chown -R 1000:1000 /src /cargo /gradle /pnpm-store /tmp/release-home; "
            "exec setpriv --reuid=1000 --regid=1000 --clear-groups bash scripts/release/build-unsigned.sh \"$1\"", "_", self.release["version"]],
            copies=[(str(self.source) + "/.", "/src")], outputs=[("/src/out/.", self.out)],
            extra=["--workdir", "/src", "--env", "HOME=/tmp/release-home", "--env", "CARGO_BUILD_JOBS=8", "--env", "RUST_TEST_THREADS=8"])
        # Stamp only the owned source copy for the image builds, never job HEAD.
        self.run(["bash", str(self.source / "scripts/release/stamp.sh"), self.release["version"]], cwd=self.source)
        return {"dev_image": dev, "smoke_sha256": digest(self.smoke_apk()),
                "fixtures_sha256": digest(self.out / "device/smoke-fixtures.tar.gz")}

    def smoke_apk(self):
        return self.out / "smoke" / f'TheVault_{self.release["version"]}_x86_64.apk'

    def unsigned_manifest(self):
        value = manifest(self.dist, self.release, False)
        atomic_json(self.out / "unsigned-manifest.json", value)
        return {"manifest_sha256": digest(self.out / "unsigned-manifest.json")}

    def sign(self):
        missing = [name for name in SIGN_ENV if name != "ANDROID_KEY_PASSWORD" and not os.environ.get(name)]
        if not os.environ.get("ANDROID_SIGNER_SHA256"):
            missing.append("ANDROID_SIGNER_SHA256 (existing shipped certificate fingerprint)")
        if missing:
            raise ReleaseError("waiting for operator signing prerequisites: " + ", ".join(missing))
        verify_manifest(self.dist, json.loads((self.out / "unsigned-manifest.json").read_text()), self.release)
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
                command(["apksigner", "sign", "--ks", str(key), "--ks-pass", "env:ANDROID_KEYSTORE_PASSWORD",
                         "--ks-key-alias", env["ANDROID_KEY_ALIAS"], "--key-pass", "env:ANDROID_KEY_PASSWORD",
                         "--out", str(output), str(aligned)], env=env)
                cert = self.run(["apksigner", "verify", "--min-sdk-version", "33", "--print-certs", str(output)]).stdout.decode()
                expected = os.environ["ANDROID_SIGNER_SHA256"].replace(":", "").lower()
                found = re.findall(r"certificate SHA-256 digest: ([0-9a-fA-F]+)", cert)
                if found != [expected]:
                    raise ReleaseError("APK signer differs from existing shipped certificate")
                badging = self.run(["aapt2", "dump", "badging", str(output)]).stdout.decode()
                code = self.run(["bash", "scripts/release/version-code.sh", version]).stdout.decode().strip()
                if f"versionCode='{code}' versionName='{version}'" not in badging:
                    raise ReleaseError("signed APK version mismatch")
            appimage = self.dist / f"TheVault_{version}_amd64.AppImage"
            command(["cargo", "tauri", "signer", "sign", "--app-version", version, str(appimage)], env=env)
            pubkey = json.loads((ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text())["plugins"]["updater"]["pubkey"]
            public, signature = self.runtime / "updater.pub", self.runtime / "updater.sig"
            public.write_bytes(base64.b64decode(pubkey))
            signature.write_bytes(base64.b64decode(Path(str(appimage) + ".sig").read_bytes()))
            self.run(["minisign", "-Vm", str(appimage), "-p", str(public), "-x", str(signature)])
            if version not in signature.read_text().split("trusted comment:")[-1]:
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
        atomic_json(self.out / "release-manifest.json", value)
        return {"manifest_sha256": digest(self.out / "release-manifest.json"), "smoke_sha256": digest(self.smoke_apk())}

    def check_assets(self):
        recorded = json.loads((self.out / "release-manifest.json").read_text())
        verify_manifest(self.dist, recorded, self.release)
        expected = self.journal.data["outputs"]["signed-manifest"]
        if digest(self.out / "release-manifest.json") != expected["manifest_sha256"] or digest(self.smoke_apk()) != expected["smoke_sha256"]:
            raise ReleaseError("signed release manifest or smoke APK changed")
        return recorded

    def images(self):
        self.check_assets()
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
                            "image_id": self.docker("image", "inspect", "--format", "{{.Id}}", tag).stdout.decode().strip()}
        return output

    def upload_files(self):
        return [*(self.dist / name for name in assets(self.release["version"])),
                self.out / "release-manifest.json", self.smoke_apk(), self.out / "device/smoke-fixtures.tar.gz"]

    def draft(self):
        self.check_assets()
        tag = "v" + self.release["version"]
        # Network/auth errors are not evidence of absence. Only a 404 is absence.
        ref = self.gh("api", f"repos/{REPO}/git/ref/tags/{tag}", check=False)
        if ref.returncode == 0:
            if json.loads(ref.stdout)["object"]["sha"] != self.release["tagged_sha"]:
                raise ReleaseError("existing release tag points at another commit")
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
        remote = {asset["name"]: asset for asset in value["assets"]}
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
            archive = self.out / image["archive"]
            if digest(archive) != image["sha256"]:
                raise ReleaseError("tested image archive changed")
            ref = REGISTRY + name + ":" + self.release["version"]
            want = self.crane("digest", "--tarball", str(archive)).stdout.decode().strip()
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
        return {"url": value["html_url"]}

    def execute(self, finish=False, evidence=None, run_id=None):
        for stage in STAGES:
            if stage in self.journal.data["completed"]:
                continue
            if stage == "smoke" and not finish:
                return "prepared: draft and version-only images await signed smoke"
            print("release stage:", stage, flush=True)
            method = getattr(self, stage.replace("-", "_"))
            result = method(evidence, run_id) if stage == "smoke" else method()
            self.journal.complete(stage, result)
        return "published and verified"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("plan", "prepare", "finish"))
    parser.add_argument("--delivery-config", type=Path, required=True)
    parser.add_argument("--state", type=Path, default=Path("release-out"))
    parser.add_argument("--version")
    parser.add_argument("--branch")
    parser.add_argument("--smoke-evidence", type=Path)
    parser.add_argument("--smoke-run-id")
    parser.add_argument("--plan-sha", help="plan only: test frozen SHA without release trailers")
    args = parser.parse_args(argv)
    config = json.loads(args.delivery_config.read_text())
    try:
        delivery_identity(config)
        with release_lock(args.state):
            if args.mode == "plan" and args.plan_sha:
                version = args.version or "0.1.14-alpha.38"
                release = identity(args.plan_sha, version, args.branch or "release/" + version.split("-")[0],
                                   "pre-release" if "-" in version else "release")
            else:
                if args.plan_sha:
                    raise ReleaseError("--plan-sha cannot authorize execution")
                # Reuse the version selected before an interrupted prepare.
                selected = args.state / "selected.json"
                if selected.exists():
                    saved = json.loads(selected.read_text())
                    if args.version and args.version != saved["version"]:
                        raise ReleaseError("requested version differs from selected release")
                    release = freeze(saved["version"], args.branch)
                    if release != saved:
                        raise ReleaseError("release identity changed since selection")
                else:
                    release = freeze(args.version, args.branch)
                    if args.mode != "plan":
                        atomic_json(selected, release)
            if args.mode == "plan":
                print(json.dumps({"stages": stage_plan(release, config), "assets": assets(release["version"])}, indent=2))
                return 0
            with private_runtime() as runtime:
                job = Release(args.state.resolve(), release, config, runtime)
                print(job.execute(args.mode == "finish", args.smoke_evidence, args.smoke_run_id))
            return 0
    except (ReleaseError, ValueError, KeyError, OSError) as error:
        # Avoid repr of objects/environment. Errors defined here contain names only.
        print("release stopped:", str(error) if isinstance(error, ReleaseError) else type(error).__name__, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

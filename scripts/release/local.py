#!/usr/bin/env python3
"""Local release identity, manifests and resumable stage journal.

The daemon owns SSH credentials. No credential values belong in this module's
configuration, state, plans or reports.
"""
from __future__ import annotations

import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import shutil


STAGES = (
    "preflight", "cut", "build", "unsigned-manifest", "sign",
    "signed-manifest", "images", "draft", "push", "smoke", "publish", "verify",
)
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-alpha\.([1-9][0-9]?))?")


class ReleaseError(Exception):
    """An actionable prerequisite or immutable-release mismatch."""


def safe_extract(tar, destination: Path) -> None:
    """Python 3.11-compatible extraction of regular files/directories only.

    Release source/checkpoints contain no links. Reject links, devices,
    traversal and excessive expansion rather than relying on tar defaults.
    """
    destination.mkdir(parents=True, exist_ok=True)
    root = destination.resolve()
    members = tar.getmembers()
    if sum(member.size for member in members) > 8 * 1024**3:
        raise ReleaseError("archive expansion exceeds 8 GiB")
    seen = set()
    for member in members:
        relative = Path(member.name)
        path = destination / relative
        if relative.is_absolute() or ".." in relative.parts or member.name in seen:
            raise ReleaseError("unsafe or duplicate archive path")
        seen.add(member.name)
        if not (member.isdir() or member.isfile()) or path.is_symlink():
            raise ReleaseError("archive links and special files are forbidden")
        if not path.resolve().is_relative_to(root):
            raise ReleaseError("archive path escapes destination")
        if member.isdir():
            path.mkdir(parents=True, exist_ok=True)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            with tar.extractfile(member) as source, path.open("wb") as output:
                shutil.copyfileobj(source, output)
            path.chmod(member.mode & 0o777)


def identity(sha: str, version: str, branch: str, kind: str) -> dict:
    match = VERSION.fullmatch(version)
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ReleaseError("release SHA must be a full lowercase commit ID")
    if not match or (match[4] and int(match[4]) > 98):
        raise ReleaseError("version must be X.Y.Z or X.Y.Z-alpha.1 through alpha.98")
    if branch != "release/" + version.split("-", 1)[0]:
        raise ReleaseError("frozen branch does not match the release version")
    if kind != ("pre-release" if match[4] else "release"):
        raise ReleaseError("release kind does not match version")
    return dict(sha=sha, version=version, branch=branch, kind=kind)


def stages(release: dict):
    if release.get("signing_mode") == "none":
        return ("preflight", "cut", "build", "manifest", "images", "draft", "push", "publish", "verify")
    return STAGES


def assets(version: str, mode="github") -> dict[str, str]:
    """Public filenames preserve the existing installed-client contract."""
    inventory = {
        f"TheVault_{version}_aarch64.apk": "android",
        f"TheVault_{version}_amd64.AppImage": "desktop",
        f"TheVault_{version}_amd64.AppImage.sig": "updater-signature",
        f"TheVault_{version}_amd64.deb": "deb",
        f"TheVault-{version}-1.x86_64.rpm": "rpm",
        "thevault-bin-aur.tar.gz": "aur",
        f"vault-{version}-linux-x86_64": "cli",
        f"vault-{version}-linux-aarch64": "cli",
        "SHA256SUMS": "checksums",
        "SHA256SUMS-cli": "checksums",
        "SHA256SUMS-android": "checksums",
        "compose.yaml": "compose",
        "Caddyfile": "compose",
        "env.example": "compose",
    }
    if mode == "none":
        return {n: r for n, r in inventory.items() if r in ("cli", "compose") or n == "SHA256SUMS-cli"}
    return inventory


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def manifest(directory: Path, release: dict, signed: bool) -> dict:
    entries = []
    expected = assets(release["version"], release.get("signing_mode", "github"))
    if not signed:
        expected = {n: r for n, r in expected.items() if r != "updater-signature"}
    for name, role in expected.items():
        path = directory / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ReleaseError(f"missing, empty or symlink asset: {name}")
        entries.append(dict(name=name, role=role, size=path.stat().st_size,
                            sha256=digest(path)))
    return dict(release=release, signed=signed, assets=entries)


def verify_manifest(directory: Path, recorded: dict, release: dict) -> None:
    if recorded["release"] != release:
        raise ReleaseError("manifest release identity mismatch")
    if manifest(directory, release, recorded["signed"]) != recorded:
        raise ReleaseError("release assets changed since manifest creation")


def compare_baseline(recorded: dict, baseline: dict) -> list[dict]:
    """Gross size regression gate, not a substitute for format/signature checks."""
    version = recorded["release"]["version"]
    expected = {row["name"].replace(baseline["version"], version): row for row in baseline["assets"]}
    if recorded["release"].get("signing_mode") == "none":
        expected = {n: r for n, r in expected.items() if n in assets(version, "none")}
    actual = {row["name"]: row for row in recorded["assets"]}
    if set(actual) != set(expected):
        raise ReleaseError("asset inventory differs from alpha.37 versioned names")
    comparisons = []
    for name, asset in actual.items():
        reference = expected[name]["size"]
        # Packaging/compression and evolving apps change sizes, but a tiny
        # stub or unexpectedly huge binary must stop before publication.
        if not reference / 2 <= asset["size"] <= reference * 2:
            raise ReleaseError("asset size outside 0.5x..2x alpha.37 baseline: " + name)
        comparisons.append({"name": name, "size": asset["size"], "baseline_size": reference})
    return comparisons


def stage_plan(release: dict) -> list[dict]:
    return [{"stage": stage, "target": (
        "thevault-android (lead-owned signed smoke)" if stage == "smoke" else
        "GitHub sign-only workflow + release-client verification (custodian switch available)" if stage == "sign" else
        "cut evidence from lead-owned target runs" if stage == "cut" else
        "DOCKER_HOST (daemon-provisioned relay)" if stage in ("build", "images") else
        "release_target (current release job)"), "sha": release["sha"], "version": release["version"],
        "dry_run": True} for stage in stages(release)]


def atomic_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, name = tempfile.mkstemp(prefix=".release-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        Path(name).unlink(missing_ok=True)


@contextlib.contextmanager
def release_lock(state_root: Path):
    """One lock across versions, from selection through final publication."""
    state_root.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (state_root / "release.lock").open("a") as stream:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise ReleaseError("another local release holds the release lock") from exc
        try:
            yield
        finally:
            fcntl.flock(stream, fcntl.LOCK_UN)


class Journal:
    """Only successful real stages advance state. Dry runs never use this class."""

    def __init__(self, path: Path, release: dict):
        self.path = path
        self.data = json.loads(path.read_text()) if path.exists() else {
            "release": release, "completed": [], "outputs": {},
        }
        if self.data["release"] != release:
            raise ReleaseError("resume SHA/version/config identity mismatch")
        completed = self.data["completed"]
        self.stages = stages(release)
        if completed != list(self.stages[:len(completed)]):
            raise ReleaseError("stage journal is not a valid completed prefix")

    def complete(self, stage: str, outputs: dict) -> None:
        if len(self.data["completed"]) >= len(self.stages) or self.stages[len(self.data["completed"])] != stage:
            raise ReleaseError("cannot skip or repeat a completed release stage")
        self.data["outputs"][stage] = outputs
        self.data["completed"].append(stage)
        atomic_json(self.path, self.data)

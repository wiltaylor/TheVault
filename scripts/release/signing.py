#!/usr/bin/env python3
"""Sign-only workflow helper and client verification of returned artifacts.

No build or publication commands. Inputs are three hash-bound unsigned files.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import zipfile

from local import ReleaseError, atomic_json, digest, safe_extract

ROOT = Path(__file__).resolve().parents[2]


def filenames(version):
    return [f"TheVault_{version}_aarch64.apk", f"TheVault_{version}_x86_64.apk",
            f"TheVault_{version}_amd64.AppImage"]


def run(argv, env=None):
    base = {k: os.environ[k] for k in ("PATH", "HOME", "JAVA_HOME", "LANG") if k in os.environ}
    result = subprocess.run(argv, env=base | (env or {}), capture_output=True)
    if result.returncode:
        raise ReleaseError(f"sign-only {Path(argv[0]).name} failed (exit {result.returncode})")
    return result.stdout


def input_manifest(directory, version, sha):
    return {"version": version, "sha": sha, "files": {
        name: {"sha256": digest(directory / name), "size": (directory / name).stat().st_size}
        for name in filenames(version)}}


def apk_contents(path):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(set(names)) != len(names):
            raise ReleaseError("APK contains duplicate ZIP entries")
        result = {}
        for name in names:
            if re.fullmatch(r"META-INF/(?:[^/]+\.(?:SF|RSA|DSA|EC)|MANIFEST\.MF)", name, re.IGNORECASE):
                continue
            with archive.open(name) as stream:
                result[name] = hashlib.file_digest(stream, "sha256").hexdigest()
        return result


def verify_outputs(unsigned, signed, version, sha, runner=run):
    expected = set(filenames(version)) | {f"TheVault_{version}_amd64.AppImage.sig", "signed-manifest.json"}
    if {path.name for path in signed.iterdir()} != expected:
        raise ReleaseError("signing artifact has missing or unexpected files")
    if any(path.is_symlink() or not path.is_file() for path in signed.iterdir()):
        raise ReleaseError("signing artifact contains links or non-files")
    report = json.loads((signed / "signed-manifest.json").read_text())
    if report["inputs"] != input_manifest(unsigned, version, sha):
        raise ReleaseError("signing artifact is for different unsigned inputs")
    expected_outputs = expected - {"signed-manifest.json"}
    if set(report["outputs"]) != expected_outputs:
        raise ReleaseError("signed output manifest inventory mismatch")
    for name in expected_outputs:
        if report["outputs"][name] != {"sha256": digest(signed / name), "size": (signed / name).stat().st_size}:
            raise ReleaseError("signed artifact hash mismatch")
    fingerprint = (ROOT / "scripts/release/android-signer.sha256").read_text().strip()
    if not re.fullmatch(r"[a-f0-9]{64}", fingerprint):
        raise ReleaseError("invalid committed Android signer pin")
    code = runner(["bash", str(ROOT / "scripts/release/version-code.sh"), version]).decode().strip()
    for name in filenames(version)[:2]:
        if apk_contents(unsigned / name) != apk_contents(signed / name):
            raise ReleaseError("APK content changed during signing")
        cert = runner(["apksigner", "verify", "--min-sdk-version", "33", "--print-certs", str(signed / name)]).decode()
        if re.findall(r"certificate SHA-256 digest: ([0-9a-fA-F]+)", cert) != [fingerprint]:
            raise ReleaseError("returned APK signer differs from committed shipped certificate")
        badging = runner(["aapt2", "dump", "badging", str(signed / name)]).decode()
        if f"versionCode='{code}' versionName='{version}'" not in badging:
            raise ReleaseError("returned APK version mismatch")
    app = filenames(version)[2]
    if digest(unsigned / app) != digest(signed / app):
        raise ReleaseError("AppImage bytes changed during signing")
    with tempfile.TemporaryDirectory(prefix="thevault-release-verify-") as directory:
        public, signature = Path(directory) / "updater.pub", Path(directory) / "updater.sig"
        pubkey = json.loads((ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text())["plugins"]["updater"]["pubkey"]
        public.write_bytes(base64.b64decode(pubkey))
        signature.write_bytes(base64.b64decode((signed / (app + ".sig")).read_bytes()))
        runner(["minisign", "-Vm", str(signed / app), "-p", str(public), "-x", str(signature)])
        if not re.search(r"(?<![\w.-])" + re.escape(version) + r"(?![\w.-])", signature.read_text().split("trusted comment:")[-1]):
            raise ReleaseError("returned AppImage signature is not version-bound")


def sign_bundle(bundle, bundle_sha, version, sha, out, tauri):
    if digest(bundle) != bundle_sha:
        raise ReleaseError("unsigned signing bundle hash mismatch")
    out.mkdir(parents=True, exist_ok=True)
    if list(out.iterdir()):
        raise ReleaseError("sign-only output directory must be empty")
    # GitHub's signing step is the only process receiving repository secrets.
    # Key files are short-lived tmpfs files, never checkout files or artifacts.
    with tempfile.TemporaryDirectory(prefix="thevault-release-sign-", dir="/dev/shm") as keys, tempfile.TemporaryDirectory(prefix="thevault-release-inputs-") as directory:
        unsigned = Path(directory)
        with tarfile.open(bundle) as archive:
            safe_extract(archive, unsigned)
        expected = set(filenames(version)) | {"input-manifest.json"}
        if {p.name for p in unsigned.iterdir()} != expected:
            raise ReleaseError("unsigned signing bundle inventory mismatch")
        inputs = input_manifest(unsigned, version, sha)
        if json.loads((unsigned / "input-manifest.json").read_text()) != inputs:
            raise ReleaseError("unsigned signing manifest mismatch")
        key = Path(keys) / "android.jks"
        key.write_bytes(base64.b64decode(os.environ["ANDROID_KEYSTORE_BASE64"], validate=True))
        key.chmod(0o600)
        env = {"ANDROID_KEYSTORE_PASSWORD": os.environ["ANDROID_KEYSTORE_PASSWORD"],
               "ANDROID_KEY_PASSWORD": os.environ.get("ANDROID_KEY_PASSWORD") or os.environ["ANDROID_KEYSTORE_PASSWORD"]}
        for name in filenames(version)[:2]:
            aligned = unsigned / "aligned.apk"
            run(["zipalign", "-f", "-p", "4", str(unsigned / name), str(aligned)])
            run(["apksigner", "sign", "--v4-signing-enabled", "false", "--ks", str(key), "--ks-pass", "env:ANDROID_KEYSTORE_PASSWORD",
                 "--ks-key-alias", os.environ["ANDROID_KEY_ALIAS"], "--key-pass", "env:ANDROID_KEY_PASSWORD",
                 "--out", str(out / name), str(aligned)], env)
        app = filenames(version)[2]
        shutil.copy2(unsigned / app, out / app)
        updater = Path(keys) / "updater.key"
        updater.write_text("".join(os.environ["TAURI_SIGNING_PRIVATE_KEY"].split()))
        updater.chmod(0o600)
        run([tauri, "signer", "sign", "-f", str(updater), "--app-version", version, str(out / app)],
            {"TAURI_SIGNING_PRIVATE_KEY_PASSWORD": os.environ["TAURI_SIGNING_PRIVATE_KEY_PASSWORD"]})
        outputs = set(filenames(version)) | {app + ".sig"}
        atomic_json(out / "signed-manifest.json", {"inputs": inputs, "outputs": {
            name: {"sha256": digest(out / name), "size": (out / name).stat().st_size} for name in outputs}})
        verify_outputs(unsigned, out, version, sha)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--bundle-sha", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--tauri-bin", required=True)
    args = parser.parse_args()
    try:
        from local import identity
        identity(args.sha, args.version, "release/" + args.version.split("-")[0], "pre-release" if "-" in args.version else "release")
        if not re.fullmatch(r"[0-9a-f]{64}", args.bundle_sha):
            raise ReleaseError("invalid signing bundle digest")
        sign_bundle(args.bundle, args.bundle_sha, args.version, args.sha, args.out, args.tauri_bin)
    except (ReleaseError, KeyError, ValueError, OSError) as error:
        raise SystemExit(str(error) if isinstance(error, ReleaseError) else "sign-only prerequisite or input failure") from None

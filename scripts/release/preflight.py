"""META-159 release-client tool contract. No network probes or secret output."""
from pathlib import Path
import json
import re

from local import ReleaseError

TOOLS = (
    ("tauri-cli", ("cargo-tauri", "--version"), "2.12.3"),
    ("Java", ("java", "-version"), "21.0.13"),
    ("jq", ("jq", "--version"), "1.7.1"),
    ("minisign", ("minisign", "-v"), "0.12"),
    ("Docker CLI", ("docker", "--version"), "29.0.3"),
    ("crane", ("crane", "version"), "0.22.1"),
    ("cosign", ("cosign", "version"), "3.1.4"),
    ("gh", ("gh", "--version"), "2.100.0"),
    ("just", ("just", "--version"), "1.59.0"),
    ("uv", ("uv", "--version"), "0.13.0"),
    ("git", ("git", "--version"), "2.47.3"),
    ("Python", ("python3", "--version"), "3.13.5"),
)


def check_workflow(gh):
    result = gh("api", "repos/wiltaylor/TheVault/actions/workflows/sign-only.yml", check=False)
    if result.returncode:
        reason = "missing" if b"404" in result.stderr else "unavailable"
        raise ReleaseError(f"release preflight: sign-only.yml is {reason}; github signing requires an active workflow. Use signing: none to omit apps, or custodian with provisioned keys")
    if json.loads(result.stdout).get("state") != "active":
        raise ReleaseError("release preflight: sign-only.yml is disabled; github signing requires an active workflow. Use signing: none to omit apps, or custodian with provisioned keys")


def check_tools(runner, android_home, mode="github"):
    if mode == "none":
        return check_common_tools(runner)
    if not android_home:
        raise ReleaseError("release preflight: ANDROID_HOME is required for build-tools 36.0.0")
    tools = Path(android_home) / "build-tools/36.0.0"
    properties = tools / "source.properties"
    if not properties.is_file() or not re.search(r"^Pkg.Revision\s*=\s*36\.0\.0\s*$", properties.read_text(), re.M):
        raise ReleaseError("release preflight: ANDROID_HOME must contain Android build-tools 36.0.0")
    commands = (*TOOLS,
                ("apksigner", (str(tools / "apksigner"), "--version"), "0.9"),
                ("aapt2", (str(tools / "aapt2"), "version"), "2.20"))
    checked = {"Android build-tools": "36.0.0"}
    for name, argv, version in commands:
        try:
            result = runner(list(argv))
        except (OSError, ReleaseError):
            raise ReleaseError(f"release preflight: missing or unusable {name}, expected {version}") from None
        output = (result.stdout + result.stderr).decode(errors="replace")
        if result.returncode or not re.search(r"(?<![0-9.])" + re.escape(version) + r"(?![0-9.])", output):
            raise ReleaseError(f"release preflight: {name} requires version {version}; installed version differs")
        checked[name] = version
    component = "Docker buildx"
    try:
        buildx = runner(["docker", "buildx", "version"])
        if buildx.returncode or not re.search(rb"buildx\s+v?[0-9]+\.[0-9]+\.[0-9]+", buildx.stdout + buildx.stderr):
            raise ReleaseError("release preflight: Docker buildx is missing or unusable")
        component = "SDK 36.0.0 zipalign"
        # zipalign has no version flag. Its SDK package identity above pins it;
        # the help banner confirms that the pinned executable can actually run.
        align = runner([str(tools / "zipalign"), "-h"], check=False)
        if align.returncode not in (0, 1, 2) or b"Zip alignment utility" not in align.stdout + align.stderr:
            raise ReleaseError("release preflight: SDK 36.0.0 zipalign is unusable")
    except (OSError, ReleaseError):
        raise ReleaseError(f"release preflight: {component} is missing or unusable") from None
    checked["buildx"] = "available (META did not specify a version pin)"
    checked["zipalign"] = "SDK 36.0.0"
    return checked


def check_common_tools(runner):
    checked = {}
    for name, argv, version in TOOLS:
        if name in ("tauri-cli", "Java", "minisign"):
            continue
        try:
            result = runner(list(argv))
        except (OSError, ReleaseError):
            raise ReleaseError(f"release preflight: missing or unusable {name}, expected {version}") from None
        if result.returncode or not re.search(rb"(?<![0-9.])" + version.encode().replace(b".", rb"\.") + rb"(?![0-9.])", result.stdout + result.stderr):
            raise ReleaseError(f"release preflight: {name} requires version {version}; installed version differs")
        checked[name] = version
    result = runner(["docker", "buildx", "version"])
    if result.returncode or not re.search(rb"buildx\s+v?[0-9]+\.[0-9]+\.[0-9]+", result.stdout + result.stderr):
        raise ReleaseError("release preflight: Docker buildx is missing or unusable")
    return checked

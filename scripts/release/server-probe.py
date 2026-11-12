"""Runs in a secret-free probe container sharing the server network namespace."""
import hashlib
import json
import sys
import time
import urllib.request

version, assets_path = sys.argv[1:]
assets = json.load(open(assets_path))
base = "http://127.0.0.1:8080"
for attempt in range(120):
    try:
        with urllib.request.urlopen(base + "/api/health", timeout=2) as response:
            if response.status == 200:
                break
    except OSError:
        time.sleep(0.5)
else:
    raise SystemExit("server image never became healthy")

def get(path):
    with urllib.request.urlopen(base + path, timeout=30) as response:
        return response.read()

assert json.loads(get("/api/version"))["server_version"] == version
clients = json.loads(get("/api/clients"))["clients"]
for asset in assets["assets"]:
    if asset["role"] not in ("android", "desktop", "cli"):
        continue
    if asset["role"] == "cli":
        path = "/api/clients/cli/linux-" + asset["name"].split("-linux-")[1]
    else:
        kind = "apk" if asset["role"] == "android" else "appimage"
        selected = [c for c in clients if c["kind"] == kind and c["version"] == version and c["build"] == "release"]
        assert len(selected) == 1
        path = "/api/clients/" + selected[0]["filename"]
    assert hashlib.sha256(get(path)).hexdigest() == asset["sha256"], asset["name"]
    print("verified image download", asset["name"])

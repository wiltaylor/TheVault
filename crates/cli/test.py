#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pytest-skillcheck"]
# ///
"""Tests for the thevault skill, crates/cli/SKILL.md, the document `vault skill` prints.

Each test starts a real vault server on port 0 in a fresh vault under the test's temporary
directory, builds and runs everything through ./dev (the host has no Rust toolchain), and puts a
`vault` wrapper first on PATH that runs the built CLI through ./dev. The agent inherits VAULT_URL
and VAULT_TOKEN, an admin token. Nothing touches the user's own server or $HOME.

Run it directly: crates/cli/test.py
"""

import json
import os
import shlex
import stat
import subprocess
import sys
import time
import urllib.request
from datetime import date, datetime, timezone
from pathlib import Path

import pytest
from skillcheck import main

REPO = Path(__file__).resolve().parents[2]
DEV = REPO / "dev"
PASSWORD = "skillcheck-password"

# The `vault` the agent runs. ./dev mounts only its working directory and passes no stdin, so the
# wrapper runs it from the test's directory, which holds the workspace, the binaries and the vault,
# feeds stdin through a file there, and copies in any file argument from outside it.
WRAPPER = r'''#!{python}
import os, shlex, shutil, subprocess, sys, tempfile
ROOT = {root!r}
DEV = {dev!r}
VAULT = os.path.join(ROOT, "opt", "vault")
args = []
for arg in sys.argv[1:]:
    path = os.path.abspath(arg)
    if os.path.isfile(path) and not path.startswith(ROOT + os.sep):
        copy = os.path.join(tempfile.mkdtemp(dir=os.path.join(ROOT, "io")), os.path.basename(path))
        shutil.copy(path, copy)
        arg = copy
    args.append(arg)
stdin = "/dev/null"
if not sys.stdin.isatty():
    fd, stdin = tempfile.mkstemp(dir=os.path.join(ROOT, "io"))
    with os.fdopen(fd, "wb") as f:
        f.write(sys.stdin.buffer.read())
cwd = os.getcwd()
env = " ".join(
    f"{{k}}={{shlex.quote(os.environ[k])}}" for k in ("VAULT_URL", "VAULT_TOKEN") if k in os.environ
)
script = (
    f"cd {{shlex.quote(cwd)}} 2>/dev/null || cd {{shlex.quote(ROOT)}}; "
    f"exec env {{env}} {{shlex.quote(VAULT)}} {{' '.join(shlex.quote(a) for a in args)}} < {{shlex.quote(stdin)}}"
)
sys.exit(subprocess.run([DEV, "sh", "-c", script], cwd=ROOT).returncode)
'''


@pytest.fixture(scope="session")
def built():
    """The server and CLI binaries, built through ./dev."""
    subprocess.run(
        [str(DEV), "cargo", "build", "-p", "cli", "-p", "server", "--bins"],
        cwd=REPO,
        check=True,
    )
    return REPO / "target" / "debug"


class Vault:
    """A running test server, and the `vault` the agent gets."""

    def __init__(self, root: Path, url: str, token: str, process: subprocess.Popen):
        self.root, self.url, self.token, self.process = root, url, token, process

    def api(self, method: str, path: str, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(self.url + path, data=data, method=method)
        request.add_header("authorization", f"Bearer {self.token}")
        if data is not None:
            request.add_header("content-type", "application/json")
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.loads(response.read() or b"null")

    def cli(self, *args: str, stdin: str = "") -> dict:
        out = subprocess.run(
            [str(self.root / "bin" / "vault"), *args],
            input=stdin,
            capture_output=True,
            text=True,
            cwd=self.root,
        )
        assert out.returncode == 0, f"vault {args} failed:\n{out.stdout}\n{out.stderr}"
        return json.loads(out.stdout)

    def type(self, name: str) -> dict | None:
        return next((t for t in self.api("GET", "/api/schema")["types"] if t["name"] == name), None)

    def note(self, id: str) -> dict:
        return self.api("GET", f"/api/notes/{id}")


@pytest.fixture
def vault(built, tmp_path, monkeypatch):
    """A fresh vault served on port 0, with an admin token in the agent's environment."""
    root = tmp_path
    for folder in ("bin", "opt", "io"):
        (root / folder).mkdir()
    for binary in ("vault", "vault-server"):
        target = root / "opt" / binary
        target.write_bytes((built / binary).read_bytes())
        target.chmod(0o755)
    wrapper = root / "bin" / "vault"
    wrapper.write_text(WRAPPER.format(python=sys.executable, root=str(root), dev=str(DEV)))
    wrapper.chmod(wrapper.stat().st_mode | stat.S_IEXEC)

    key = root / "secret.key"
    folder = root / "vault"
    opt = root / "opt"
    stop = root / "stop"
    subprocess.run(
        [
            str(DEV),
            "sh",
            "-c",
            f"printf '%s\\n' {shlex.quote(PASSWORD)} | VAULT_SECRET_KEY_FILE={key} "
            f"{opt}/vault init {folder} --password-stdin",
        ],
        cwd=root,
        check=True,
        capture_output=True,
    )
    process = subprocess.Popen(
        [
            str(DEV),
            "sh",
            "-c",
            # Stopping the docker client leaves the container running, so the server stops when
            # the stop file appears, or after two hours if the test never gets to write it.
            f"VAULT_SECRET_KEY_FILE={key} {opt}/vault-server --vault {folder} --listen 127.0.0.1:0 & "
            f"for i in $(seq 14400); do [ -e {stop} ] && break; sleep 0.5; done; kill $!; wait",
        ],
        cwd=root,
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        listen = json.loads(process.stdout.readline())["listen"]
        url = f"http://{listen}"
        # The first token: log in with the password, then create a token as the Access page does.
        login = urllib.request.Request(
            url + "/api/login",
            data=json.dumps({"password": PASSWORD}).encode(),
            headers={"content-type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(login, timeout=60) as response:
            cookie = response.headers["set-cookie"].split(";")[0]
        create = urllib.request.Request(
            url + "/api/tokens",
            data=json.dumps({"name": "skillcheck", "scope": "admin"}).encode(),
            headers={"content-type": "application/json", "cookie": cookie},
            method="POST",
        )
        with urllib.request.urlopen(create, timeout=60) as response:
            token = json.loads(response.read())["secret"]

        monkeypatch.setenv("PATH", f"{root / 'bin'}{os.pathsep}{os.environ['PATH']}")
        monkeypatch.setenv("VAULT_URL", url)
        monkeypatch.setenv("VAULT_TOKEN", token)
        monkeypatch.delenv("VAULT_DIR", raising=False)
        yield Vault(root, url, token, process)
    finally:
        stop.touch()
        try:
            process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()


TASK_TYPE = """label = "Task"

field status {
  kind    = "select"
  options = ["todo", "doing", "done"]
}

field due {
  kind = "date"
}

template default {
  tab "Task" {
    slot "title"
    slot "status"
    slot "due"
  }
}
"""

INVOICE_TYPE = """label  = "Invoice"
plural = "Invoices"

field vendor {
  kind = "string"
}

field amount {
  kind = "float"
}

field received {
  kind = "date"
}

template default {
  tab "Invoice" {
    slot "title"
    slot "vendor"
    slot "amount"
    slot "received"
  }
}
"""


def test_adds_a_type_with_labelled_values(run_skill, vault):
    before = {t["name"] for t in vault.api("GET", "/api/schema")["types"]}

    result = run_skill("add a contact type with a list of phone numbers that each have a label")

    assert result.exit_code == 0, result.stderr[-2000:]
    assert not result.reached_home()
    new = [t for t in vault.api("GET", "/api/schema")["types"] if t["name"] not in before]
    assert new, f"no type was created:\n{result.output[-2000:]}"
    labelled = []
    for t in new:
        fields = {f["name"]: f for f in t["fields"]}
        for f in t["fields"]:
            target = fields.get(f.get("labels") or "")
            if f["kind"] == "list" and target and target["kind"] == "list" and target.get("items") == "string":
                labelled.append(f"{t['name']}.{f['name']}")
    assert labelled, f"no list field has labels naming a list of strings: {new}"


def test_adds_a_field_to_a_type_with_notes(run_skill, vault):
    vault.cli("type", "put", "task", stdin=TASK_TYPE)
    seeded = [
        vault.cli("put", "--type", "task", stdin=f'title = "{title}"\nstatus = "{status}"\ndue = "{due}"\n')
        for title, status, due in [
            ("Renew the passport", "todo", "2026-12-01"),
            ("Fix the gate", "doing", "2026-11-15"),
            ("Book the dentist", "done", "2026-10-30"),
        ]
    ]

    result = run_skill("add a priority field to my task type")

    assert result.exit_code == 0, result.stderr[-2000:]
    assert not result.reached_home()
    task = vault.type("task")
    assert task and any(f["name"] == "priority" for f in task["fields"]), task
    for note in seeded:
        now = vault.note(note["id"])["note"]["fields"]
        for name, value in note["note"]["fields"].items():
            if name != "updated":
                assert now.get(name) == value, f"{note['id']}.{name}: {value!r} became {now.get(name)!r}"
    assert vault.api("GET", "/api/flagged")["notes"] == [], "notes no longer fit their type"


def test_stamps_new_invoices_and_schedules_a_reminder(run_skill, vault):
    vault.cli("type", "put", "invoice", stdin=INVOICE_TYPE)
    vault.cli("put", "--type", "invoice", stdin='title = "March power bill"\nvendor = "PowerCo"\namount = 182.4\n')
    shipped = {t["name"] for t in vault.api("GET", "/api/tasks")["tasks"]}

    result = run_skill(
        "every new invoice should get today's date stamped in received, "
        "and remind me every Monday 9am to review invoices"
    )

    assert result.exit_code == 0, result.stderr[-2000:]
    assert not result.reached_home()

    # A note created after the run gets today's date from a handler.
    invoice = vault.type("invoice")
    events = {h["event"] for h in invoice["handlers"]}
    assert events & {"after_create", "before_create"}, f"no create handler: {invoice['handlers']}"
    created = vault.api(
        "POST", "/api/notes", {"type": "invoice", "wcl": 'title = "April water bill"\namount = 64.0\n'}
    )
    today = {date.today().isoformat(), datetime.now(timezone.utc).date().isoformat()}
    for _ in range(60):
        received = vault.note(created["id"])["note"]["fields"].get("received")
        if received:
            break
        time.sleep(0.5)
    assert received in today, f"received is {received!r}, not today"

    # A Monday 09:00 task whose run succeeds and notifies the user.
    tasks = [t for t in vault.api("GET", "/api/tasks")["tasks"] if t["name"] not in shipped]
    weekly = [t for t in tasks if t["schedule"].split()[:2] == ["0", "9"] and t["schedule"].split()[4] in ("1", "MON", "mon")]
    assert weekly, f"no task runs Mondays at 09:00: {[(t['name'], t['schedule']) for t in tasks]}"
    task = weekly[0]["name"]

    def notified():
        items = vault.api("GET", "/api/notifications?limit=100")["items"]
        return sum(1 for n in items if n["source"] == f"task:{task}")

    before = notified()
    run = vault.api("POST", f"/api/tasks/{task}/run")
    assert run["outcome"] == "succeeded", run
    assert notified() > before, "the task's run added no notification"


if __name__ == "__main__":
    raise SystemExit(main(__file__))

"""Disposable loopback-only vault. Never reads a production vault or environment credentials."""
import http.cookiejar
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import socketserver
import subprocess
import sys
import time
import urllib.error
import urllib.request

CONTROL = "/tmp/staging.sock"
BASE = "http://127.0.0.1:8080"


class Fake:
    def __init__(self, name, *args):
        self.proc = subprocess.Popen(
            ["testkit-fake-" + name, *args], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, text=True, bufsize=1,
        )
        self.info = json.loads(self.proc.stdout.readline())

    def tell(self, line):
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()
        answer = json.loads(self.proc.stdout.readline())
        if "error" in answer:
            raise RuntimeError(answer["error"])
        return answer


class Api:
    def __init__(self, password):
        self.client = urllib.request.build_opener(
            urllib.request.ProxyHandler({}),
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
        )
        self.send("POST", "/api/login", {"password": password})

    def send(self, method, path, data=None):
        text = isinstance(data, str)
        body = data.encode() if text else json.dumps(data).encode() if data is not None else None
        req = urllib.request.Request(BASE + path, data=body, method=method,
                                     headers={"Content-Type": "text/plain" if text else "application/json"})
        try:
            with self.client.open(req, timeout=60) as res:
                body = res.read()
                return json.loads(body) if body else None
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"{method} {path}: {error.code} {error.read().decode()}") from None

    def secret(self, name, value):
        self.send("PUT", "/api/secrets/" + name, {"value": value})


def account(api, name, kind, url, key, model):
    existing = api.send("GET", "/api/integrations/ai")["accounts"]
    record = {"name": name, "kind": kind, "base_url": url, "model": model,
              "headers": [], "context": None}
    found = next((a for a in existing if a["name"] == name), None)
    if found:
        ident = found["id"]
        api.send("PUT", "/api/integrations/ai/accounts/" + ident, record)
    else:
        ident = api.send("POST", "/api/integrations/ai/accounts", record)["id"]
    api.send("PUT", f"/api/integrations/ai/accounts/{ident}/key", {"key": key})
    return ident


def seed(api, fakes):
    ids = {}
    for name, kind, suffix, model in [
        ("openai", "openai_compatible", "/v1", "fake-model"),
        ("anthropic", "anthropic", "", "claude-opus-5"),
        ("decisions", "openai_compatible", "", "typesafe/jev-1.13"),
    ]:
        info = fakes[name].info
        ids[name] = account(api, "Staging " + name, kind, info["base_url"] + suffix,
                            info["api_key"], model)
    api.send("PUT", "/api/integrations/ai/default", {"id": ids["anthropic"]})
    api.secret("google_settings", json.dumps({"endpoints": fakes["google"].info["endpoints"],
                                              "gmail_labels": ["INBOX"]}))
    fakes["google"].tell("add-message " + json.dumps({
        "from": "Alex Synthetic <alex@example.invalid>", "to": ["staging@example.invalid"],
        "subject": "Synthetic meeting", "text": "Fabricated mail. Meeting at 10 tomorrow.",
        "labels": ["INBOX"], "at": int(time.time() * 1000),
    }))
    for name, value in (("google_client_id", "fake-client-id.apps.googleusercontent.com"),
                        ("google_client_secret", "fake-client-secret"),
                        ("google_refresh_token", "fake-refresh-token")):
        api.secret(name, value)
    api.send("PUT", "/api/tasks/google_gmail", '''label = "Synthetic Gmail import"
schedule = "*/10 * * * *"
script = "google_gmail"
timeout = "PT1M"
retries = 0
paused = true
allow_domains = ["''' + fakes["google"].info["host"] + '''"]
secrets = ["google_client_id", "google_client_secret", "google_refresh_token", "google_settings", "google_gmail_state", "google_gmail_trashed"]
''')
    imported = api.send("POST", "/api/tasks/google_gmail/run?wait=true")
    assert imported["outcome"] == "succeeded", imported
    # Notes are explicitly session-created fixtures, not device-ingestion evidence.
    api.send("PUT", "/api/types/staging_sms", {"wcl": '''label = "Synthetic SMS"
template default {
  tab "Main" {
    slot "title"
  }
}
'''})
    api.send("POST", "/api/notes?type=staging_sms", {"note": {"fields": {
        "title": "Synthetic SMS from +15550100001: meeting at 10 tomorrow"}}})
    api.send("PUT", "/api/agents/staging_reader", '''label = "Synthetic staging reader"
permissions {
  types = ["staging_sms"]
  tools = ["get_note"]
}
trigger received {
  on = ["create"]
  type = "staging_sms"
  prompt = "Read this synthetic SMS. Do not change or send anything."
}
''')
    return ids


def smoke(api, fakes):
    # No default route, no veth, no DNS or reachable real provider, even for a copied agent.
    interfaces = sorted(p.name for p in Path("/sys/class/net").iterdir())
    assert interfaces == ["lo"], interfaces
    for host in ("1.1.1.1", "8.8.8.8"):
        with socket.socket() as conn:
            conn.settimeout(1)
            assert conn.connect_ex((host, 443)) != 0, "real outbound connection possible"
    emails = api.send("POST", "/api/query", {
        "query": 'from email include archived where title == "Synthetic meeting"', "page_size": 10})
    assert emails["rows"], "fabricated Gmail was not imported"
    previous = api.send("GET", "/api/agents/staging_reader/runs")
    assert previous["tools"] == ["get_note"], previous["tools"]
    old_runs = {r["id"] for r in previous["triggers"][0]["runs"]}
    before = fakes["anthropic"].tell("requests")["count"]
    fakes["anthropic"].tell('push {"text":"Synthetic SMS read. No actions taken."}')
    api.send("POST", "/api/notes?type=staging_sms", {"note": {"fields": {
        "title": "Synthetic SMS trigger fixture " + secrets.token_hex(4)}}})
    until = time.monotonic() + 100
    while time.monotonic() < until:
        runs = api.send("GET", "/api/agents/staging_reader/runs")
        recent = runs["triggers"][0]["runs"]
        if any(r["id"] not in old_runs and r["outcome"] == "succeeded" for r in recent):
            assert fakes["anthropic"].tell("requests")["count"] > before
            return {"passed": 5, "checks": ["loopback only", "real outbound blocked", "Gmail imported",
                                            "read-only tools", "fake trigger succeeded"]}
        time.sleep(1)
    raise RuntimeError("trigger did not succeed: " + json.dumps(runs))


def serve():
    os.umask(0o077)
    children = []
    def cleanup():
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
        for child in children:
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
    def stop(*_):
        cleanup()
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        password_path = Path("/config/password")
        if not password_path.exists():
            password_path.write_text(secrets.token_urlsafe(32))
        password = password_path.read_text()
        if not Path("/data/.vault/state.db").exists():
            subprocess.run(["vault", "init", "/data", "--password-stdin"],
                           input=password + "\n", text=True, check=True, stdout=subprocess.DEVNULL)
        fakes = {}
        for name in ("google", "openai", "anthropic", "decisions"):
            fake = Fake(name, *(["chat_completions"] if name == "openai" else []))
            fakes[name] = fake
            children.append(fake.proc)
        server = subprocess.Popen(["vault-server", "--vault", "/data", "--listen", "127.0.0.1:8080"])
        children.append(server)
        until = time.monotonic() + 90
        while True:
            try:
                api = Api(password)
                break
            except urllib.error.URLError:
                if time.monotonic() > until or server.poll() is not None:
                    raise
                time.sleep(0.5)
        seed(api, fakes)

        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                try:
                    command = json.loads(self.rfile.readline())
                    if command[0] == "health":
                        assert all(p.poll() is None for p in children)
                        result = api.send("GET", "/api/health")
                    elif command[0] == "smoke":
                        result = smoke(api, fakes)
                    elif command[0] == "tell":
                        result = fakes[command[1]].tell(command[2])
                    elif command[0] == "api":
                        result = api.send(command[1], command[2], json.loads(command[3]) if len(command) > 3 else None)
                    else:
                        raise ValueError("expected health, smoke, tell or api")
                    answer = {"ok": result}
                except Exception as error:
                    answer = {"error": str(error)}
                self.wfile.write(json.dumps(answer).encode() + b"\n")
        Path(CONTROL).unlink(missing_ok=True)
        with socketserver.UnixStreamServer(CONTROL, Handler) as control:
            print("staging ready: fake-only, network disabled", flush=True)
            control.serve_forever()
    finally:
        cleanup()


def main():
    if sys.argv[1:] == ["serve"]:
        serve()
    else:
        with socket.socket(socket.AF_UNIX) as conn:
            conn.connect(CONTROL)
            conn.sendall(json.dumps(sys.argv[1:]).encode() + b"\n")
            answer = json.loads(conn.makefile().readline())
        print(json.dumps(answer))
        if "error" in answer:
            raise SystemExit(1)


if __name__ == "__main__":
    main()

// What the S6 tests run the desktop app against: a testkit home server on port 0, a private D-Bus
// session with an unlocked throwaway Secret Service keyring, and tauri-driver. Every path lives
// in one temporary folder, and every port is picked free.

import { execFileSync, spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { waitFor } from "./webdriver.mjs";

export const repo = fileURLToPath(new URL("../../..", import.meta.url));

function binary(env, path) {
  const found = process.env[env] ?? join(repo, path);
  if (!existsSync(found)) throw new Error(`${found} is missing; run \`just desktop-test\`, or set ${env}.`);
  return found;
}

export const appBinary = () => binary("THEVAULT_DESKTOP_BIN", "apps/desktop/src-tauri/target/debug/thevault");

/** A port nothing listens on now. */
export function freePort() {
  return new Promise((resolve, reject) => {
    const s = createServer();
    s.once("error", reject);
    s.listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

/** The first line `child` prints, within `timeout` ms. */
function firstLine(child, what, timeout = 60_000) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${what} printed nothing`)), timeout);
    const lines = createInterface({ input: child.stdout });
    lines.once("line", (line) => {
      clearTimeout(timer);
      resolve(line);
      lines.on("line", () => {});
    });
    child.once("exit", (code) => reject(new Error(`${what} exited with ${code}`)));
  });
}

function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  return new Promise((resolve) => {
    child.once("exit", () => resolve());
    child.kill("SIGTERM");
    setTimeout(() => child.kill("SIGKILL"), 5_000).unref();
  });
}

/** A vault-server on a seeded vault, which can be stopped and started again on the same port. */
export class HomeServer {
  constructor(root) {
    this.vault = join(root, "home-vault");
    mkdirSync(join(this.vault, "types"), { recursive: true });
    mkdirSync(join(this.vault, "taxonomies"));
    copyFileSync(join(repo, "crates/cli/starter/types/note.wcl"), join(this.vault, "types/note.wcl"));
    copyFileSync(join(repo, "crates/cli/starter/taxonomies/tags.wcl"), join(this.vault, "taxonomies/tags.wcl"));
    const seeded = JSON.parse(
      execFileSync(binary("TESTKIT_SEED_BIN", "target/debug/testkit-seed"), [this.vault], { encoding: "utf8" }),
    );
    this.password = seeded.password;
    this.adminToken = seeded.admin_token;
    this.child = null;
    this.address = "127.0.0.1:0";
  }

  get url() {
    return `http://${this.address}`;
  }

  async start() {
    this.child = spawn(
      binary("VAULT_SERVER_BIN", "target/debug/vault-server"),
      ["--vault", this.vault, "--listen", this.address],
      { stdio: ["ignore", "pipe", "inherit"] },
    );
    this.address = JSON.parse(await firstLine(this.child, "vault-server")).listen;
  }

  stop() {
    return stop(this.child);
  }

  async api(method, path, body) {
    const res = await fetch(`${this.url}${path}`, {
      method,
      headers: { authorization: `Bearer ${this.adminToken}`, "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await res.text();
    if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${text}`);
    return JSON.parse(text);
  }
}

/**
 * The desktop session the app runs in: its own XDG folders, a private D-Bus session and an
 * unlocked gnome-keyring as its Secret Service, and tauri-driver on free ports.
 */
export class Desktop {
  constructor(root) {
    this.root = root;
    const dir = (name) => {
      const path = join(root, name);
      mkdirSync(path, { recursive: true, mode: 0o700 });
      return path;
    };
    this.env = {
      ...process.env,
      HOME: dir("home"),
      XDG_DATA_HOME: dir("data"),
      XDG_CONFIG_HOME: dir("config"),
      XDG_CACHE_HOME: dir("cache"),
      XDG_RUNTIME_DIR: dir("runtime"),
    };
    this.children = [];
    this.namePasswdEntry();
  }

  /**
   * ./dev runs as the host's uid, which has no passwd entry in the container, and dbus-daemon
   * refuses connections from a user it cannot look up. nss_wrapper gives the session one.
   */
  namePasswdEntry() {
    const uid = process.getuid();
    const gid = process.getgid();
    try {
      execFileSync("getent", ["passwd", String(uid)], { stdio: "ignore" });
      return;
    } catch {}
    const wrapper = "/usr/lib/x86_64-linux-gnu/libnss_wrapper.so";
    if (!existsSync(wrapper)) throw new Error(`uid ${uid} has no passwd entry and ${wrapper} is missing; run through ./dev`);
    const passwd = join(this.root, "passwd");
    const group = join(this.root, "group");
    writeFileSync(passwd, `root:x:0:0:root:/root:/bin/sh\ntester:x:${uid}:${gid}:tester:${this.env.HOME}:/bin/sh\n`);
    writeFileSync(group, `root:x:0:\ntester:x:${gid}:\n`);
    Object.assign(this.env, { LD_PRELOAD: wrapper, NSS_WRAPPER_PASSWD: passwd, NSS_WRAPPER_GROUP: group });
  }

  /** Where the app keeps its local vault and device file. */
  get dataDir() {
    return join(this.env.XDG_DATA_HOME, "thevault");
  }

  spawn(cmd, args, options = {}) {
    const child = spawn(cmd, args, { env: this.env, stdio: ["ignore", "pipe", "inherit"], ...options });
    this.children.push(child);
    return child;
  }

  async start() {
    const bus = this.spawn("dbus-daemon", ["--session", "--nofork", "--print-address=1"]);
    this.env.DBUS_SESSION_BUS_ADDRESS = (await firstLine(bus, "dbus-daemon")).trim();

    const keyring = this.spawn("gnome-keyring-daemon", ["--foreground", "--unlock", "--components=secrets"], {
      stdio: ["pipe", "ignore", "inherit"],
    });
    keyring.stdin.end("throwaway keyring password");
    await waitFor("the Secret Service on the session bus", () => {
      const out = execFileSync(
        "dbus-send",
        [
          "--session",
          "--print-reply",
          "--dest=org.freedesktop.DBus",
          "/org/freedesktop/DBus",
          "org.freedesktop.DBus.NameHasOwner",
          "string:org.freedesktop.secrets",
        ],
        { env: this.env, encoding: "utf8" },
      );
      return out.includes("boolean true");
    });

    const port = await freePort();
    const nativePort = await freePort();
    this.spawn("tauri-driver", ["--port", String(port), "--native-port", String(nativePort)], {
      stdio: ["ignore", "inherit", "inherit"],
    });
    this.webdriver = `http://127.0.0.1:${port}`;
    await waitFor("tauri-driver", async () => (await fetch(`${this.webdriver}/status`)).ok);
  }

  async stop() {
    for (const child of this.children.reverse()) await stop(child);
  }
}

export function tempRoot() {
  return mkdtempSync(join(tmpdir(), "thevault-desktop-e2e-"));
}

export function removeRoot(root) {
  rmSync(root, { recursive: true, force: true });
}

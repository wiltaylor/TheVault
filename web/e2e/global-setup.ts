import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { BULK_NOTES, BULK_TYPE, TASK_TYPE, bulkId, bulkTitle } from "./vault";

const web = fileURLToPath(new URL("..", import.meta.url));
const repo = join(web, "..");

function binary(env: string, name: string): string {
  const path = process.env[env] ?? join(repo, "target", "debug", name);
  if (!existsSync(path)) {
    throw new Error(`${path} is missing. Run \`cargo build -p server -p testkit --bins\`, or set ${env}.`);
  }
  return path;
}

/** A fresh vault in a temporary folder: the test types and BULK_NOTES notes of `bulk`. */
function seedVault(): string {
  const vault = mkdtempSync(join(tmpdir(), "thevault-e2e-"));
  mkdirSync(join(vault, "types"));
  writeFileSync(join(vault, "types/task.wcl"), TASK_TYPE);
  writeFileSync(join(vault, "types/bulk.wcl"), BULK_TYPE);
  mkdirSync(join(vault, "notes/bulk"), { recursive: true });
  for (let i = 1; i <= BULK_NOTES; i++) {
    writeFileSync(
      join(vault, "notes/bulk", `${bulkId(i)}.wcl`),
      `title   = "${bulkTitle(i)}"\ncreated = "2026-10-12T09:15:00Z"\nupdated = "2026-10-12T09:15:00Z"\n`,
    );
  }
  return vault;
}

/** The address vault-server prints as its first stdout line. */
function listenAddress(server: ChildProcess): Promise<string> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("vault-server did not report its address")), 60_000);
    const lines = createInterface({ input: server.stdout! });
    lines.once("line", (line) => {
      clearTimeout(timer);
      try {
        resolve(JSON.parse(line).listen);
      } catch (e) {
        reject(new Error(`unexpected first line from vault-server: ${line}`));
      }
      // Keep draining stdout so the server never blocks on a full pipe.
      lines.on("line", () => {});
    });
    server.once("exit", (code) => reject(new Error(`vault-server exited with ${code}`)));
  });
}

// Starts vault-server on a seeded temporary vault and port 0, serving web/dist, and hands the
// URL, credentials and vault folder to the tests through the environment (S5).
export default async function globalSetup() {
  if (!existsSync(join(web, "dist/index.html"))) {
    throw new Error("web/dist is missing. Run `pnpm build` first.");
  }
  const vault = seedVault();
  const seeded = JSON.parse(
    execFileSync(binary("TESTKIT_SEED_BIN", "testkit-seed"), [vault], { encoding: "utf8" }),
  ) as { password: string; admin_token: string };

  const server = spawn(
    binary("VAULT_SERVER_BIN", "vault-server"),
    ["--vault", vault, "--listen", "127.0.0.1:0", "--web", join(web, "dist")],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  let address: string;
  try {
    address = await listenAddress(server);
  } catch (e) {
    server.kill();
    rmSync(vault, { recursive: true, force: true });
    throw e;
  }

  process.env.E2E_BASE_URL = `http://${address}`;
  process.env.E2E_PASSWORD = seeded.password;
  process.env.E2E_ADMIN_TOKEN = seeded.admin_token;
  process.env.E2E_VAULT = vault;
  return async () => {
    server.kill();
    rmSync(vault, { recursive: true, force: true });
  };
}

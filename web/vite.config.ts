import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";

/** The workspace version in the root `Cargo.toml`, which the server shares: one source for both. */
function workspaceVersion(): string {
  const cargo = readFileSync(fileURLToPath(new URL("../Cargo.toml", import.meta.url)), "utf8");
  const section = cargo.split(/^\[workspace\.package\]$/m)[1] ?? "";
  const version = /^version\s*=\s*"([^"]+)"/m.exec(section)?.[1];
  if (!version) throw new Error("the root Cargo.toml names no [workspace.package] version");
  return version;
}

/** The short commit the build is made from: `THEVAULT_WEB_COMMIT`, else git's, else null. */
function commit(): string | null {
  const given = process.env.THEVAULT_WEB_COMMIT?.trim();
  if (given) return given;
  try {
    const out = execFileSync("git", ["rev-parse", "--short", "HEAD"], {
      cwd: fileURLToPath(new URL(".", import.meta.url)),
      stdio: ["ignore", "pipe", "ignore"],
    });
    return out.toString().trim() || null;
  } catch {
    return null;
  }
}

export default defineConfig({
  plugins: [solid()],
  define: {
    __WEB_VERSION__: JSON.stringify(workspaceVersion()),
    __WEB_COMMIT__: JSON.stringify(commit()),
  },
  build: {
    target: "es2022",
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["./src/test-setup.ts"],
  },
});

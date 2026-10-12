import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preview } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));

// Serves the built app on port 0 and hands the URL to the tests through the
// environment, so parallel runs never fight over a port.
export default async function globalSetup() {
  if (!existsSync(`${root}/dist/index.html`)) {
    throw new Error("web/dist is missing. Run `pnpm build` first.");
  }
  const server = await preview({
    root,
    logLevel: "warn",
    preview: { host: "127.0.0.1", port: 0, strictPort: true, open: false },
  });
  const url = server.resolvedUrls?.local[0];
  if (!url) {
    await server.close();
    throw new Error("vite preview did not report a URL");
  }
  process.env.E2E_BASE_URL = url;
  return async () => {
    await server.close();
  };
}

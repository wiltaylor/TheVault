import { test as shared } from "./fixtures";
import globalSetup from "./global-setup";

// Specs that change server-wide secrets, tasks or shipped schemas cannot share the
// suite's vault. Each worker (including repeat workers) owns a server and vault.
// Auto worker fixtures run before the baseURL and admin fixtures read the environment.
export const test = shared.extend<{}, { isolatedVault: void }>({
  isolatedVault: [async ({}, use) => {
    const names = ["E2E_BASE_URL", "E2E_PASSWORD", "E2E_ADMIN_TOKEN", "E2E_VAULT"];
    const previous = names.map((name) => process.env[name]);
    const stop = await globalSetup();
    try {
      await use();
    } finally {
      await stop();
      names.forEach((name, i) => {
        if (previous[i] === undefined) delete process.env[name];
        else process.env[name] = previous[i];
      });
    }
  }, { scope: "worker", auto: true, timeout: 120_000 }],
});

export { expect, logIn } from "./fixtures";

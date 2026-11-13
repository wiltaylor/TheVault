import { readFileSync } from "node:fs";
import { join } from "node:path";
import { aiAccounts, startFake, useAccount } from "./ai";
import { expect, logIn, test, vaultDir } from "./fixtures";

test("Settings selects classifier accounts without provider calls or default changes", async ({ page, admin }) => {
  const fake = await startFake("TESTKIT_FAKE_OPENAI_BIN", "testkit-fake-openai");
  try {
    const beforeDefault = (await aiAccounts(admin)).default;
    const name = "Classifier fixture";
    const id = await useAccount(admin, { name, kind: "openai_compatible", base_url: `${new URL(fake.fake.base_url).origin}/api/v1` });
    const before = new Map(["agent_safety", "scam"].map((n) => [n, readFileSync(join(vaultDir(), `classifiers/${n}.wcl`), "utf8")]));
    await logIn(page, "/settings/ai");
    for (const classifier of ["agent_safety", "scam"]) {
      const picker = page.getByLabel(`Account for ${classifier}`, { exact: true });
      await picker.selectOption(id);
      await expect(picker).toBeEnabled();
      await expect(page.getByRole("region", { name: `${classifier} classifier`, exact: true })).toContainText(`Resolved account: ${name}`);
      const after = readFileSync(join(vaultDir(), `classifiers/${classifier}.wcl`), "utf8");
      const source = before.get(classifier)!;
      expect(after).toBe(source.includes("account =") ? source.replace(/account = "[^"]*"/, `account = "${name}"`) : `account = "${name}"\n${source}`);
    }
    expect((await aiAccounts(admin)).default).toBe(beforeDefault);
    expect((await fake.tell("requests")).count).toBe(0);
  } finally { fake.stop(); }
});

import { type ChildProcess, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { useAccount } from "./ai";
import { expect, logIn, test } from "./fixtures";

// Script tools (script-tools T2): create a tool with its script on Settings > Tools, run it from
// the page with a param and see what it returned, then the Assistant calls it in chat through the
// fake Anthropic API, and the call is recorded in the activity log with the version that ran.

const repo = fileURLToPath(new URL("../..", import.meta.url));

function binary(env: string, name: string): string {
  const path = process.env[env] ?? join(repo, "target", "debug", name);
  if (!existsSync(path)) throw new Error(`${path} is missing. Run \`cargo build -p server -p testkit --bins\`.`);
  return path;
}

interface Fake {
  base_url: string;
  api_key: string;
}

let proc: ChildProcess;
let fake: Fake;
const answers: ((line: string) => void)[] = [];

/** Send a command to the fake Anthropic API and wait until it has taken effect. */
async function tell(line: string): Promise<Record<string, unknown>> {
  const answer = new Promise<string>((resolve) => answers.push(resolve));
  proc.stdin!.write(`${line}\n`);
  const got = JSON.parse(await answer) as Record<string, unknown>;
  expect(got.error).toBeUndefined();
  return got;
}

test.beforeAll(async () => {
  proc = spawn(binary("TESTKIT_FAKE_ANTHROPIC_BIN", "testkit-fake-anthropic"), [], { stdio: ["pipe", "pipe", "inherit"] });
  fake = await new Promise<Fake>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("the fake did not report its address")), 30_000);
    const lines = createInterface({ input: proc.stdout! });
    lines.once("line", (line) => {
      clearTimeout(timer);
      resolve(JSON.parse(line) as Fake);
      lines.on("line", (answer) => answers.shift()?.(answer));
    });
    proc.once("exit", (code) => reject(new Error(`the fake exited with ${code}`)));
  });
});

test.afterAll(() => {
  proc?.kill();
});

test("a tool made on the Tools page runs from the page and an agent calls it in chat", async ({ page, admin }) => {
  const tag = Date.now().toString(36);
  const name = `greet_${tag}`;
  const account = `Fake Anthropic ${new URL(fake.base_url).port}`;
  await useAccount(admin, { name: account, kind: "anthropic", base_url: fake.base_url, key: fake.api_key, model: "claude-opus-5" });

  // Create it: the starter names a script of the tool's own name, written in the same save.
  await logIn(page, "/settings/tools/new");
  await page.getByLabel("Name").fill(name);
  await expect(page.getByLabel("Tool file")).toHaveValue(new RegExp(`script\\s+= "${name}"`));
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page).toHaveURL(new RegExp(`/settings/tools/${name}$`));
  await expect(page.getByRole("status")).toHaveText("Saved.");
  const stored = (await (await admin.get(`/api/scripts/${name}`)).json()) as { source: string };
  expect(stored.source).toContain("fn run(params: Value)");

  // Run it from the page.
  const form = page.getByRole("form", { name: "Run the tool" });
  await form.getByLabel("name *").fill("World");
  await form.getByRole("button", { name: "Run" }).click();
  const result = page.getByTestId("tool-run-result");
  await expect(result).toContainText("Succeeded");
  await expect(result.getByLabel("Result")).toContainText('"Hello, World"');

  // It is listed.
  await page.getByRole("link", { name: "Tools", exact: true }).first().click();
  await expect(page.getByRole("table", { name: "Script tools" }).getByRole("link", { name })).toBeVisible();

  // The Assistant calls it, then answers.
  await tell(`push ${JSON.stringify({ tools: [{ id: "toolu_greet", name, input: { name: "Ada" } }] })}`);
  await tell(`push ${JSON.stringify({ text: `The tool said hello to Ada ${tag}.` })}`);
  await page.goto("/agents/assistant");
  const chat = page.locator(".agent-chat");
  await chat.getByLabel("Account").selectOption({ label: account });
  await expect(chat.getByLabel("Model")).toHaveValue("claude-opus-5");
  await chat.getByRole("textbox", { name: "Ask AI" }).fill(`Greet Ada ${tag}`);
  await chat.getByRole("textbox", { name: "Ask AI" }).press("Enter");
  await expect(chat.getByText(`The tool said hello to Ada ${tag}.`)).toBeVisible();
  await expect(chat.getByText(`Called ${name}`)).toBeVisible();
  await expect(chat.locator("details.ai-step", { hasText: `Called ${name}` })).toContainText("(ok");

  // The run is in the activity log, with the version of the script that ran.
  await expect
    .poll(async () => {
      const log = (await (await admin.get("/api/activity?category=ai")).json()) as { items: { kind: string; detail?: Record<string, unknown> }[] };
      const run = log.items.find((i) => i.kind === "ai.script" && i.detail?.tool === name);
      return run ? { outcome: run.detail?.outcome, kind: (run.detail?.script as { kind?: string } | undefined)?.kind } : null;
    })
    .toEqual({ outcome: "ok", kind: "vault" });
});

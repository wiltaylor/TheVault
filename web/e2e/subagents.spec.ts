import { type Running, startFake, useAccount } from "./ai";
import { expect, logIn, test } from "./fixtures";

// Sub agents (subagents U9) against the fake Anthropic API, through an account of the worker's
// own chosen at the chat's foot: an agent starts two sub agents in one round, a throwaway one and
// one delegated to a named agent; both run at once and finish, and the agent answers with what
// they found. Each call is a block in the chat; clicking it opens the sub agent's session beside
// the chat, read only, in the one pane beside it. The named agent's proposal is applied from the parent chat, as that agent,
// and the sessions are not in the session picker. Each fake reply is for the one request whose
// body holds its text, since the sub agents ask at the same time.

let anthropic: Running;
let fakeAccount: string;

test.beforeAll(async () => {
  anthropic = await startFake("TESTKIT_FAKE_ANTHROPIC_BIN", "testkit-fake-anthropic");
  fakeAccount = `Fake Anthropic ${new URL(anthropic.fake.base_url).port}`;
});

test.afterAll(() => {
  anthropic?.stop();
});

/** Queue a reply for the request whose body holds `when`. */
function reply(when: string, spec: Record<string, unknown>) {
  return anthropic.tell(`push ${JSON.stringify({ ...spec, when })}`);
}

test("an agent's sub agents run together, their sessions open beside the chat, and a proposal applies from it", async ({ page, admin }) => {
  await useAccount(admin, {
    name: fakeAccount,
    kind: "anthropic",
    base_url: anthropic.fake.base_url,
    key: anthropic.fake.api_key,
    model: "claude-opus-5",
  });
  const tag = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const lead = `lead_${tag}`;
  const writer = `writer_${tag}`;
  for (const [name, wcl] of [
    [writer, `label = "Writer ${tag}"\nsystem_prompt = "You write invitations."\n`],
    [lead, `label = "Lead ${tag}"\n\npermissions {\n  subagents {\n    allow  = true\n    agents = ["${writer}"]\n  }\n}\n`],
  ] as const) {
    const put = await admin.put(`/api/agents/${name}`, { data: { wcl } });
    expect(put.status(), await put.text()).toBe(200);
  }

  const venue = `Find a venue ${tag}`;
  const invite = `Write the invite ${tag}`;
  const start = (id: string, task: string, agent: string | null) => ({
    id,
    name: "start_subagent",
    input: { task, instructions: null, agent, permissions: null, model: null },
  });
  await reply(`Plan the party ${tag}`, { tools: [start("toolu_venue", venue, null), start("toolu_invite", invite, writer)] });
  // The throwaway sub agent has the task in its prompt; the named one in its first message.
  await reply(`Your task: ${venue}`, { text: `The hall is free ${tag}.`, delay_ms: 20 });
  const note = JSON.stringify({ fields: { title: `Invite ${tag}` } });
  await reply(`Task: ${invite}`, { tools: [{ id: "toolu_note", name: "propose_create", input: { type: "note", note } }] });
  await reply("toolu_note", { text: `Drafted the invite ${tag}.` });
  await reply(`The hall is free ${tag}`, { text: `Booked the hall and drafted the invite ${tag}.` });

  await logIn(page, `/agents/${lead}`);
  const chat = page.locator(".agent-chat");
  await chat.getByLabel("Account").selectOption({ label: fakeAccount });
  await expect(chat.getByLabel("Model")).toHaveValue("claude-opus-5");
  await chat.getByLabel("Ask AI").fill(`Plan the party ${tag}`);
  await chat.getByLabel("Ask AI").press("Enter");

  const conversation = chat.getByRole("list", { name: "Conversation" });
  await expect(conversation).toContainText(`Booked the hall and drafted the invite ${tag}.`);
  const throwaway = conversation.getByRole("region", { name: `Sub agent: ${venue}` });
  const delegated = conversation.getByRole("region", { name: `Delegated to Writer ${tag}: ${invite}` });
  await expect(throwaway.getByRole("status").first()).toHaveText("done");
  await expect(delegated.getByRole("status").first()).toHaveText("done");
  await throwaway.getByText("Result").click();
  await expect(throwaway).toContainText(`The hall is free ${tag}.`);
  expect((await anthropic.tell("requests")).count).toBe(5);

  // The two sessions overlapped: both started before either ended.
  const threads = await admin.get(`/api/agents/${lead}/threads`);
  const parentId = ((await threads.json()) as { threads: { id: string }[] }).threads[0]!.id;
  const parent = (await (await admin.get(`/api/agents/${lead}/threads/${parentId}`)).json()) as {
    subagents: { thread: string; agent: string; started_ms: number; ended_ms: number }[];
  };
  expect(parent.subagents).toHaveLength(2);
  const [a, b] = parent.subagents;
  expect(a!.started_ms).toBeLessThanOrEqual(b!.ended_ms);
  expect(b!.started_ms).toBeLessThanOrEqual(a!.ended_ms);

  // The named agent's proposal stands under its block, and applies from here as that agent.
  const card = delegated.getByRole("region", { name: `Proposed new note Invite ${tag}` });
  await card.getByRole("button", { name: "Apply" }).click();
  await expect(card.getByText("Created.")).toBeVisible();
  const href = await card.getByRole("link", { name: "Open the new note" }).getAttribute("href");
  const id = decodeURIComponent(href!.split("/").pop()!);
  const history = (await (await admin.get(`/api/notes/${id}/history`)).json()) as { versions: { author: { kind: string; name: string } }[] };
  expect(history.versions[0]!.author).toEqual({ kind: "agent", name: writer });

  // Each block opens its session beside the chat, read only.
  await throwaway.getByRole("link", { name: `Sub agent: ${venue}` }).click();
  const venueSession = page.getByRole("region", { name: "Sub agent session" }).filter({ hasText: `The hall is free ${tag}.` });
  await expect(venueSession).toBeVisible();
  await expect(venueSession.getByLabel("Ask AI")).toHaveCount(0);
  await delegated.getByRole("link", { name: `Delegated to Writer ${tag}: ${invite}` }).click();
  const inviteSession = page.getByRole("region", { name: "Sub agent session" }).filter({ hasText: `Drafted the invite ${tag}.` });
  await expect(inviteSession).toBeVisible();
  await expect(inviteSession).toContainText(`Proposed new note Invite ${tag}: applied`);
  await expect(inviteSession.getByRole("button", { name: "Apply" })).toHaveCount(0);
  // The pane beside shows one session at a time: the second block took over the first's pane.
  await expect(page.locator(".pane").filter({ visible: true })).toHaveCount(2);
  await expect(page.getByRole("region", { name: "Sub agent session" }).filter({ visible: true })).toHaveCount(1);

  // The sessions are listed only under their parent: not in the session picker, nor the writer's.
  await chat.getByRole("button", { name: /^Chat session: / }).click();
  const list = chat.getByRole("listbox", { name: "Chats" });
  await expect(list.getByRole("option", { name: new RegExp(`Plan the party ${tag}`) })).toBeVisible();
  await expect(list.getByRole("option")).toHaveCount(2);
  await expect(list.getByRole("option", { name: new RegExp(venue) })).toHaveCount(0);
  const writers = (await (await admin.get(`/api/agents/${writer}/threads`)).json()) as { threads: unknown[] };
  expect(writers.threads).toHaveLength(0);
  expect(parent.subagents.find((s) => s.agent === writer)).toBeTruthy();

  await page.keyboard.press("Escape");
  for (const name of [lead, writer]) expect((await admin.delete(`/api/agents/${name}`)).ok()).toBe(true);
});

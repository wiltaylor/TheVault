import { Route, Router } from "@solidjs/router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AiAccountStatus, AiStatus } from "../api";

const aiStatus = vi.fn();
const aiSaveSettings = vi.fn();
const aiCreateAccount = vi.fn();
const aiUpdateAccount = vi.fn();
const aiDeleteAccount = vi.fn();
const aiOrderAccounts = vi.fn();
const aiSetDefault = vi.fn();
const aiPutKey = vi.fn();
const aiTest = vi.fn();
const aiModels = vi.fn();
const chatgptStart = vi.fn();
const chatgptFinish = vi.fn();
const chatgptDisconnect = vi.fn();
const app = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    api: {
      ...real.api,
      aiStatus,
      aiSaveSettings,
      aiCreateAccount,
      aiUpdateAccount,
      aiDeleteAccount,
      aiOrderAccounts,
      aiSetDefault,
      aiPutKey,
      aiTest,
      aiModels,
      chatgptStart,
      chatgptFinish,
      chatgptDisconnect,
      app,
    },
  };
});

const { Ai, CHATGPT_USAGE_URL, testText, untilText, usageText, uniqueName } = await import("./Ai");
const { SettingsLayout, WEB_APP_ONLY } = await import("./SettingsLayout");
const { forgetAppInfo } = await import("../inApp");

const KINDS: AiStatus["kinds"] = [
  { kind: "anthropic", label: "Anthropic API", default_url: "https://api.anthropic.com", default_window: 200000 },
  { kind: "openai", label: "OpenAI API", default_url: "https://api.openai.com", default_window: 400000 },
  { kind: "openai_compatible", label: "OpenAI-compatible", default_url: null, default_window: 128000 },
  { kind: "chatgpt", label: "ChatGPT subscription", default_url: null, default_window: 272000 },
];

function account(over: Partial<AiAccountStatus> & Pick<AiAccountStatus, "id" | "name" | "kind">): AiAccountStatus {
  const kind = KINDS.find((k) => k.kind === over.kind)!;
  return {
    base_url: null,
    model: null,
    headers: [],
    context: null,
    configured: false,
    key_set: false,
    signed_in: null,
    kind_label: kind.label,
    default_base_url: kind.default_url,
    default_window: kind.default_window,
    window: over.context?.window ?? kind.default_window,
    agents: [],
    usage: null,
    ...over,
  };
}

const CLAUDE = account({ id: "a1", name: "Claude", kind: "anthropic", model: "claude-opus-5", configured: true, key_set: true });
const ROUTER = account({
  id: "r1",
  name: "OpenRouter",
  kind: "openai_compatible",
  base_url: "https://openrouter.ai/api/v1",
  headers: [["X-Title", "TheVault"]],
  configured: true,
});

function status(accounts: AiAccountStatus[] = [], def: string | null = accounts[0]?.id ?? null): AiStatus {
  return {
    secrets_key: true,
    accounts,
    default: def,
    kinds: KINDS,
    confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true, agents: true, skills: true },
    web: { search: true, read: true, download: true },
    context: { auto_compact: true, compact_at: 80 },
  };
}

const ANTHROPIC_MODELS = {
  account: "a1",
  model: "claude-opus-5",
  models: [
    { id: "claude-opus-5", display_name: "Claude Opus 5" },
    { id: "claude-sonnet-5", display_name: "Claude Sonnet 5" },
  ],
};

const AUTH_URL = "https://auth.openai.com/api/accounts/authorize?client_id=dynamic_agent_client&state=s1";

beforeEach(() => {
  aiModels.mockResolvedValue(ANTHROPIC_MODELS);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.restoreAllMocks();
  forgetAppInfo();
});

/** Wait for the accounts card to show. */
async function ready() {
  await screen.findByRole("region", { name: "Accounts" });
}

function region(name: string) {
  return within(screen.getByRole("region", { name }));
}

function row(name: string) {
  return within(screen.getByRole("row", { name }));
}

describe("testText", () => {
  it("names the model on success and the kind on failure", () => {
    expect(testText({ ok: true, account: "a1", model: "claude-opus-5", ms: 12, text: "OK", error: null })).toBe(
      "OK from claude-opus-5 in 12 ms",
    );
    expect(
      testText({ ok: false, account: "o1", model: "gpt-6", ms: 3, text: null, error: { kind: "auth", message: "OpenAI refused the key" } }),
    ).toBe("Failed (auth): OpenAI refused the key");
  });
});

describe("usage text (accounts A13)", () => {
  it("says how much is used and when it resets", () => {
    const now = 1_000_000_000;
    expect(untilText(now + 130 * 60_000, now)).toBe("2 h 10 min");
    expect(untilText(now + 59 * 60_000 + 1, now)).toBe("1 h");
    expect(untilText(now + 5 * 60_000, now)).toBe("5 min");
    expect(untilText(now + (3 * 24 + 4) * 3_600_000, now)).toBe("3 d 4 h");
    expect(untilText(now - 1, now)).toBe("now");
    expect(usageText({ label: "5 h", used_percent: 41.6, window_minutes: 300, resets_at_ms: now + 130 * 60_000 }, now)).toBe(
      "42% used, resets in 2 h 10 min",
    );
    expect(usageText({ label: "Weekly", used_percent: 3, window_minutes: null, resets_at_ms: null }, now)).toBe("3% used");
  });

  it("makes a name unique without regard to case", () => {
    expect(uniqueName("OpenAI API", [{ name: "Claude" }])).toBe("OpenAI API");
    expect(uniqueName("OpenAI API", [{ name: "openai api" }, { name: "OpenAI API 2" }])).toBe("OpenAI API 3");
  });
});

describe("Settings > AI accounts (accounts A11)", () => {
  it("lists the accounts with their kind, model, state and the default", async () => {
    const chatgpt = account({ id: "c1", name: "ChatGPT", kind: "chatgpt", configured: true, signed_in: { account: "me@example.com", expired: false } });
    const expired = account({ id: "c2", name: "Old plan", kind: "chatgpt", signed_in: { account: null, expired: true } });
    aiStatus.mockResolvedValue({ ...status([CLAUDE, ROUTER, chatgpt, expired], "r1"), secrets_key: false });
    render(() => <Ai />);
    await ready();
    expect(screen.getByRole("alert")).toHaveTextContent("This server has no secrets key");
    const rows = within(screen.getByRole("table", { name: "AI accounts" }))
      .getAllByRole("row")
      .filter((r) => r.hasAttribute("data-account"))
      .map((r) => [...r.querySelectorAll("td")].slice(0, 5).map((td) => td.textContent));
    expect(rows).toEqual([
      ["Claude", "Anthropic API", "claude-opus-5", "Key set", "Make default"],
      ["OpenRouter", "OpenAI-compatible", "Kind default", "No key", "Default"],
      ["ChatGPT", "ChatGPT subscription", "Kind default", "Signed in as me@example.com", "Make default"],
      ["Old plan", "ChatGPT subscription", "Kind default", "Sign-in expired", "Make default"],
    ]);
    expect(row("Old plan").getByRole("button", { name: "Test Old plan" })).toBeDisabled();
    expect(row("Claude").getByRole("button", { name: "Move Claude up" })).toBeDisabled();
    expect(row("Old plan").getByRole("button", { name: "Move Old plan down" })).toBeDisabled();
  });

  it("says there is no account yet", async () => {
    aiStatus.mockResolvedValue(status());
    render(() => <Ai />);
    await ready();
    expect(screen.getByText("No AI account yet. Add one to use the chat and agents.")).toBeTruthy();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("adds an Anthropic account: the kind first, then its fields, and the key never comes back", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE]));
    const made = account({ id: "a2", name: "Anthropic API", kind: "anthropic", base_url: "https://api.z.ai/api/anthropic", configured: false });
    aiCreateAccount.mockResolvedValue({ ...made, configured: true, key_set: true });
    aiStatus.mockResolvedValueOnce(status([CLAUDE])).mockResolvedValue(status([CLAUDE, { ...made, configured: true, key_set: true }]));
    render(() => <Ai />);
    await ready();

    fireEvent.click(screen.getByRole("button", { name: "Add account" }));
    const form = region("Add an account");
    expect(form.queryByLabelText("Name")).toBeNull();
    expect(form.getByRole("button", { name: "Add account" })).toBeDisabled();
    const kinds = form.getByLabelText("Kind") as HTMLSelectElement;
    expect([...kinds.options].map((o) => o.textContent)).toEqual([
      "Choose a kind",
      "Anthropic API",
      "OpenAI API",
      "OpenAI-compatible",
      "ChatGPT subscription",
    ]);
    fireEvent.change(kinds, { target: { value: "anthropic" } });
    expect(form.getByLabelText("Name")).toHaveValue("Anthropic API");
    expect(form.getByLabelText("Base URL")).toHaveAttribute("placeholder", "https://api.anthropic.com");
    expect(form.getByLabelText("Context window, tokens")).toHaveAttribute("placeholder", "200000");
    expect(form.queryByText("Extra headers")).toBeNull();
    const key = form.getByLabelText("API key") as HTMLInputElement;
    expect(key.type).toBe("password");
    expect(key.placeholder).toBe("Paste a key");
    fireEvent.input(form.getByLabelText("Base URL"), { target: { value: " https://api.z.ai/api/anthropic " } });
    fireEvent.input(key, { target: { value: " sk-ant-secret " } });
    fireEvent.input(form.getByLabelText("Model"), { target: { value: "glm-5" } });
    fireEvent.click(form.getByRole("button", { name: "Add account" }));

    await waitFor(() =>
      expect(aiCreateAccount).toHaveBeenCalledWith({
        kind: "anthropic",
        name: "Anthropic API",
        base_url: "https://api.z.ai/api/anthropic",
        model: "glm-5",
        headers: [],
        context: null,
        key: "sk-ant-secret",
      }),
    );
    expect(aiPutKey).not.toHaveBeenCalled();
    // The form is the new account's now.
    const edit = within(await screen.findByRole("region", { name: "Edit Anthropic API" }));
    expect((edit.getByLabelText("API key") as HTMLInputElement).value).toBe("");
    expect(edit.getByLabelText("API key")).toHaveAttribute("placeholder", "set");
    expect(document.body.textContent).not.toContain("sk-ant-secret");
    expect(row("Anthropic API").getByText("Key set")).toBeTruthy();
  });

  it("adds an OpenAI-compatible account with a base URL, extra headers and no key", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE]));
    const made = account({ id: "r2", name: "Z.ai", kind: "openai_compatible", configured: true });
    aiCreateAccount.mockResolvedValue(made);
    render(() => <Ai />);
    await ready();

    fireEvent.click(screen.getByRole("button", { name: "Add account" }));
    const form = region("Add an account");
    fireEvent.change(form.getByLabelText("Kind"), { target: { value: "openai_compatible" } });
    expect(form.getByLabelText("Name")).toHaveValue("OpenAI-compatible");
    expect(form.getByLabelText("API key")).toHaveAttribute("placeholder", "Optional: some services need none");
    fireEvent.input(form.getByLabelText("Name"), { target: { value: "Z.ai" } });
    // The base URL is required.
    fireEvent.click(form.getByRole("button", { name: "Add account" }));
    expect(await form.findByRole("alert")).toHaveTextContent("An OpenAI-compatible account needs its base URL.");
    expect(aiCreateAccount).not.toHaveBeenCalled();
    fireEvent.input(form.getByLabelText("Base URL"), { target: { value: "https://api.z.ai/api/paas/v4" } });
    fireEvent.input(form.getByLabelText("Context window, tokens"), { target: { value: "64000" } });

    expect(form.getByText("No extra headers.")).toBeTruthy();
    fireEvent.click(form.getByRole("button", { name: "Add header" }));
    fireEvent.click(form.getByRole("button", { name: "Add header" }));
    fireEvent.click(form.getByRole("button", { name: "Add header" }));
    fireEvent.input(form.getByLabelText("Header 1 name"), { target: { value: "HTTP-Referer" } });
    fireEvent.input(form.getByLabelText("Header 1 value"), { target: { value: "https://vault.example" } });
    fireEvent.input(form.getByLabelText("Header 2 name"), { target: { value: "X-Drop" } });
    fireEvent.input(form.getByLabelText("Header 3 name"), { target: { value: " X-Title " } });
    fireEvent.input(form.getByLabelText("Header 3 value"), { target: { value: "TheVault" } });
    fireEvent.click(form.getByRole("button", { name: "Remove header 2" }));
    expect(form.getByLabelText("Header 2 name")).toHaveValue(" X-Title ");
    fireEvent.click(form.getByRole("button", { name: "Add account" }));

    await waitFor(() =>
      expect(aiCreateAccount).toHaveBeenCalledWith({
        kind: "openai_compatible",
        name: "Z.ai",
        base_url: "https://api.z.ai/api/paas/v4",
        model: null,
        headers: [
          ["HTTP-Referer", "https://vault.example"],
          ["X-Title", "TheVault"],
        ],
        context: { window: 64000 },
      }),
    );
    expect(aiPutKey).not.toHaveBeenCalled();
  });

  it("edits an account: renames it, picks the model from the live list and saves it in one", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE, ROUTER]));
    aiUpdateAccount.mockResolvedValue(status([{ ...CLAUDE, name: "Work Claude", model: "claude-sonnet-5" }, ROUTER]));
    render(() => <Ai />);
    await ready();

    fireEvent.click(row("Claude").getByRole("button", { name: "Edit Claude" }));
    const form = region("Edit Claude");
    expect(form.getByText("Anthropic API")).toBeTruthy();
    expect(form.queryByLabelText("Kind")?.tagName).not.toBe("SELECT");
    await waitFor(() => expect(form.getByLabelText("Model").tagName).toBe("SELECT"));
    expect(aiModels).toHaveBeenCalledWith("a1");
    const model = form.getByLabelText("Model") as HTMLSelectElement;
    expect([...model.options].map((o) => o.textContent)).toEqual([
      "Default (claude-opus-5)",
      "Claude Opus 5 (claude-opus-5)",
      "Claude Sonnet 5 (claude-sonnet-5)",
    ]);
    expect(model.value).toBe("claude-opus-5");
    fireEvent.change(model, { target: { value: "claude-sonnet-5" } });
    fireEvent.input(form.getByLabelText("Name"), { target: { value: "Work Claude" } });
    fireEvent.click(form.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(aiUpdateAccount).toHaveBeenCalledWith("a1", {
        name: "Work Claude",
        base_url: null,
        model: "claude-sonnet-5",
        headers: [],
        context: null,
      }),
    );
    expect(aiPutKey).not.toHaveBeenCalled();
    expect(await screen.findByRole("region", { name: "Edit Work Claude" })).toBeTruthy();
    expect(row("Work Claude").getByText("claude-sonnet-5")).toBeTruthy();
  });

  it("keeps an OpenAI-compatible account's headers in its form, and types the model when the list fails", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE, ROUTER]));
    const { ApiError } = await import("../api");
    aiModels.mockRejectedValue(new ApiError(502, { error: "upstream", message: "the service answered 404" }));
    render(() => <Ai />);
    await ready();
    fireEvent.click(row("OpenRouter").getByRole("button", { name: "Edit OpenRouter" }));
    const form = region("Edit OpenRouter");
    expect(form.getByLabelText("Base URL")).toHaveValue("https://openrouter.ai/api/v1");
    expect(form.getByLabelText("Header 1 name")).toHaveValue("X-Title");
    expect(form.getByLabelText("Header 1 value")).toHaveValue("TheVault");
    expect(await form.findByText(/The model list could not be read \(the service answered 404\): type a model id/)).toBeTruthy();
    expect(form.getByLabelText("Model").tagName).toBe("INPUT");
  });

  it("removes a key after confirming", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE]));
    aiPutKey.mockResolvedValue(status([{ ...CLAUDE, key_set: false, configured: false }]));
    const ask = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(() => <Ai />);
    await ready();
    fireEvent.click(row("Claude").getByRole("button", { name: "Edit Claude" }));
    fireEvent.click(region("Edit Claude").getByRole("button", { name: "Remove key" }));
    expect(ask).toHaveBeenCalledWith("Remove the API key of Claude? TheVault forgets it.");
    await waitFor(() => expect(aiPutKey).toHaveBeenCalledWith("a1", ""));
    expect(await row("Claude").findByText("Not set up")).toBeTruthy();
  });

  it("tests an account and shows how it went", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE, ROUTER]));
    aiTest
      .mockResolvedValueOnce({ ok: true, account: "a1", model: "claude-opus-5", ms: 42, text: "OK", error: null })
      .mockResolvedValueOnce({ ok: false, account: "r1", model: null, ms: 7, text: null, error: { kind: "unsupported", message: "this service does not speak Chat Completions" } });
    render(() => <Ai />);
    await ready();
    fireEvent.click(row("Claude").getByRole("button", { name: "Test Claude" }));
    await waitFor(() => expect(aiTest).toHaveBeenCalledWith("a1"));
    expect(await screen.findByText("Claude: OK from claude-opus-5 in 42 ms")).toBeTruthy();
    fireEvent.click(row("OpenRouter").getByRole("button", { name: "Test OpenRouter" }));
    expect(await screen.findByText("OpenRouter: Failed (unsupported): this service does not speak Chat Completions")).toBeTruthy();
    // A Test with no model chosen saves the preselected one, so the status is read again.
    await waitFor(() => expect(aiStatus).toHaveBeenCalledTimes(2));
  });

  it("makes an account the default and reorders the table", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE, ROUTER], "a1"));
    aiSetDefault.mockResolvedValue(status([CLAUDE, ROUTER], "r1"));
    aiOrderAccounts.mockResolvedValue(status([ROUTER, CLAUDE], "r1"));
    render(() => <Ai />);
    await ready();
    expect(row("Claude").getByText("Default")).toBeTruthy();
    fireEvent.click(row("OpenRouter").getByRole("button", { name: "Make OpenRouter the default" }));
    await waitFor(() => expect(aiSetDefault).toHaveBeenCalledWith("r1"));
    expect(await row("Claude").findByRole("button", { name: "Make Claude the default" })).toBeTruthy();
    fireEvent.click(row("Claude").getByRole("button", { name: "Move Claude down" }));
    await waitFor(() => expect(aiOrderAccounts).toHaveBeenCalledWith(["r1", "a1"]));
    await waitFor(() =>
      expect(
        screen
          .getAllByRole("row")
          .filter((r) => r.hasAttribute("data-account"))
          .map((r) => r.getAttribute("aria-label")),
      ).toEqual(["OpenRouter", "Claude"]),
    );
  });

  it("deletes an account after a confirm that names the agents using it", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE, { ...ROUTER, agents: ["triage", "digest"] }]));
    aiDeleteAccount.mockResolvedValue(status([CLAUDE]));
    const ask = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValueOnce(true);
    render(() => <Ai />);
    await ready();
    fireEvent.click(row("OpenRouter").getByRole("button", { name: "Delete OpenRouter" }));
    await waitFor(() =>
      expect(ask).toHaveBeenCalledWith("Delete OpenRouter? The agents triage, digest name it and will use the default account instead."),
    );
    expect(aiDeleteAccount).not.toHaveBeenCalled();
    fireEvent.click(row("OpenRouter").getByRole("button", { name: "Delete OpenRouter" }));
    await waitFor(() => expect(aiDeleteAccount).toHaveBeenCalledWith("r1"));
    await waitFor(() => expect(screen.queryByRole("row", { name: "OpenRouter" })).toBeNull());
  });

  it("deletes an account no agent names with a plain confirm", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE, ROUTER]));
    aiDeleteAccount.mockResolvedValue(status([ROUTER]));
    const ask = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(() => <Ai />);
    await ready();
    fireEvent.click(row("Claude").getByRole("button", { name: "Delete Claude" }));
    await waitFor(() => expect(aiDeleteAccount).toHaveBeenCalledWith("a1"));
    expect(ask).toHaveBeenCalledWith("Delete Claude? TheVault forgets its key or sign-in.");
  });
});

describe("Settings > AI ChatGPT accounts", () => {
  const plan = account({ id: "c1", name: "ChatGPT subscription", kind: "chatgpt" });

  it("adds a ChatGPT account, then signs it in through the pasted address", async () => {
    aiStatus.mockResolvedValueOnce(status([CLAUDE])).mockResolvedValue(status([CLAUDE, plan]));
    aiCreateAccount.mockResolvedValue(plan);
    chatgptStart.mockResolvedValue({ auth_url: AUTH_URL });
    const signedIn = { ...plan, configured: true, signed_in: { account: "me@example.com", expired: false } };
    chatgptFinish.mockResolvedValue(status([CLAUDE, signedIn]));
    render(() => <Ai />);
    await ready();

    fireEvent.click(screen.getByRole("button", { name: "Add account" }));
    const form = region("Add an account");
    fireEvent.change(form.getByLabelText("Kind"), { target: { value: "chatgpt" } });
    expect(form.queryByLabelText("Base URL")).toBeNull();
    expect(form.queryByLabelText("API key")).toBeNull();
    expect(form.getByText("Add the account, then sign in with ChatGPT.")).toBeTruthy();
    fireEvent.click(form.getByRole("button", { name: "Add account" }));
    await waitFor(() =>
      expect(aiCreateAccount).toHaveBeenCalledWith({
        kind: "chatgpt",
        name: "ChatGPT subscription",
        base_url: null,
        model: null,
        headers: [],
        context: null,
      }),
    );

    const edit = within(await screen.findByRole("region", { name: "Edit ChatGPT subscription" }));
    fireEvent.click(edit.getByRole("button", { name: "Continue with ChatGPT" }));
    await waitFor(() => expect(chatgptStart).toHaveBeenCalledWith("c1"));
    const link = await edit.findByRole("link", { name: "Open the ChatGPT sign-in page" });
    expect(link.getAttribute("href")).toBe(AUTH_URL);
    const pasted = "http://127.0.0.1:53683/auth/callback?code=abc&state=s1";
    fireEvent.input(edit.getByLabelText("Address from the browser"), { target: { value: ` ${pasted} ` } });
    fireEvent.click(edit.getByRole("button", { name: "Finish signing in" }));
    await waitFor(() => expect(chatgptFinish).toHaveBeenCalledWith("c1", pasted));
    expect(await screen.findByText("Using a ChatGPT plan as me@example.com")).toBeTruthy();
    expect(row("ChatGPT subscription").getByText("Signed in as me@example.com")).toBeTruthy();
    expect(screen.getByRole("link", { name: "Manage usage" }).getAttribute("href")).toBe(CHATGPT_USAGE_URL);
  });

  it("keeps the paste box and shows the refusal when finishing fails", async () => {
    aiStatus.mockResolvedValue(status([plan]));
    chatgptStart.mockResolvedValue({ auth_url: AUTH_URL });
    const { ApiError } = await import("../api");
    chatgptFinish.mockRejectedValue(new ApiError(400, { error: "bad_state", message: "The sign-in expired: start again." }));
    render(() => <Ai />);
    await ready();
    fireEvent.click(row("ChatGPT subscription").getByRole("button", { name: "Edit ChatGPT subscription" }));
    const edit = region("Edit ChatGPT subscription");
    fireEvent.click(edit.getByRole("button", { name: "Continue with ChatGPT" }));
    fireEvent.input(await edit.findByLabelText("Address from the browser"), { target: { value: "http://127.0.0.1:53683/auth/callback?code=x" } });
    fireEvent.click(edit.getByRole("button", { name: "Finish signing in" }));
    expect(await edit.findByText("The sign-in expired: start again.")).toBeTruthy();
    expect(edit.getByLabelText("Address from the browser")).toBeTruthy();
  });

  it("disconnects a ChatGPT account", async () => {
    const signedIn = { ...plan, configured: true, signed_in: { account: "me@example.com", expired: false } };
    aiStatus.mockResolvedValue(status([signedIn]));
    chatgptDisconnect.mockResolvedValue(status([plan]));
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(() => <Ai />);
    await ready();
    fireEvent.click(row("ChatGPT subscription").getByRole("button", { name: "Edit ChatGPT subscription" }));
    fireEvent.click(region("Edit ChatGPT subscription").getByRole("button", { name: "Disconnect" }));
    await waitFor(() => expect(chatgptDisconnect).toHaveBeenCalledWith("c1"));
    expect(await region("Edit ChatGPT subscription").findByRole("button", { name: "Continue with ChatGPT" })).toBeTruthy();
  });

  it("shows the plan's usage windows as bars, and a Test reads them again", async () => {
    const now = Date.now();
    const usage = {
      read_at_ms: now - 60_000,
      plan: "Plus",
      windows: [
        { label: "5 hours", used_percent: 41.6, window_minutes: 300, resets_at_ms: now + 130 * 60_000 + 30_000 },
        { label: "Weekly", used_percent: 7, window_minutes: 10080, resets_at_ms: null },
      ],
    };
    const signedIn = { ...plan, configured: true, signed_in: { account: "me@example.com", expired: false }, usage };
    aiStatus
      .mockResolvedValueOnce(status([CLAUDE, signedIn]))
      .mockResolvedValue(
        status([CLAUDE, { ...signedIn, usage: { ...usage, windows: [{ ...usage.windows[0]!, used_percent: 50 }, usage.windows[1]!] } }]),
      );
    aiTest.mockResolvedValue({ ok: true, account: "c1", model: "gpt-6", ms: 30, text: "OK", error: null });
    render(() => <Ai />);
    await ready();
    const bars = within(screen.getByRole("group", { name: "ChatGPT subscription usage" }));
    expect(bars.getByRole("meter", { name: "5 hours used" })).toHaveAttribute("aria-valuenow", "42");
    expect(bars.getByText("42% used, resets in 2 h 11 min")).toBeTruthy();
    expect(bars.getByText("7% used")).toBeTruthy();
    expect(bars.getByText(/^Plus plan, as of /)).toBeTruthy();
    // API-key accounts show none.
    expect(screen.queryByRole("group", { name: "Claude usage" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Test ChatGPT subscription" }));
    await waitFor(() => expect(aiTest).toHaveBeenCalledWith("c1"));
    expect(await screen.findByText(/^50% used, resets in/)).toBeTruthy();
  });
});

describe("the Context card", () => {
  it("saves the compaction switch and share", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE]));
    aiSaveSettings.mockResolvedValue({ ...status([CLAUDE]), context: { auto_compact: false, compact_at: 80 } });
    render(() => <Ai />);
    await ready();
    const c = region("Context");
    fireEvent.click(c.getByLabelText("Compact long chats on their own"));
    await waitFor(() => expect(aiSaveSettings).toHaveBeenCalledWith({ context: { auto_compact: false } }));
    await waitFor(() => expect(c.getByLabelText("Compact at, percent of the window")).toBeDisabled());
  });
});

describe("the chat settings moved to Assistant (agents G6)", () => {
  it("links to Assistant's agent screen in place of the system prompt and switch cards", async () => {
    aiStatus.mockResolvedValue(status([CLAUDE]));
    render(() => <Ai />);
    await ready();
    const link = screen.getByRole("link", { name: "Assistant agent" });
    expect(link.getAttribute("href")).toBe("/agents/assistant");
    expect(link.parentElement).toHaveTextContent("The chat panel's prompt and permissions are on the Assistant agent.");
    expect(screen.queryByLabelText("System prompt")).toBeNull();
    expect(screen.queryByRole("switch")).toBeNull();
  });
});

describe("the AI section", () => {
  function layout() {
    return render(() => (
      <Router>
        <Route path="*" component={() => <SettingsLayout>page</SettingsLayout>} />
      </Router>
    ));
  }

  it("is a Settings section in the web app", async () => {
    app.mockResolvedValue(null);
    layout();
    const link = await screen.findByRole("link", { name: "AI" });
    expect(link.getAttribute("href")).toBe("/settings/ai");
    expect(WEB_APP_ONLY).toContain("/settings/ai");
  });

  it("is absent in the desktop and Android apps", async () => {
    app.mockResolvedValue({ app: true, platform: "android" });
    layout();
    await waitFor(() => expect(screen.queryByRole("link", { name: "Integrations" })).toBeNull());
    expect(screen.queryByRole("link", { name: "AI" })).toBeNull();
    expect(screen.getByRole("link", { name: "Devices" })).toBeTruthy();
  });
});

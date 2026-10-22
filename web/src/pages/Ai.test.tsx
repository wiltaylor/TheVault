import { Route, Router } from "@solidjs/router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AiStatus } from "../api";

const aiStatus = vi.fn();
const aiSaveSettings = vi.fn();
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

const { Ai, CHATGPT_USAGE_URL, testText } = await import("./Ai");
const { SettingsLayout, WEB_APP_ONLY } = await import("./SettingsLayout");
const { forgetAppInfo } = await import("../inApp");

function status(over: Partial<Record<"anthropic" | "openai" | "chatgpt", object>> = {}, def: AiStatus["default_provider"] = null): AiStatus {
  return {
    secrets_key: true,
    default_provider: def,
    providers: {
      anthropic: { configured: false, model: null, ...over.anthropic },
      openai: { configured: false, model: null, ...over.openai },
      chatgpt: { configured: false, model: null, account: null, ...over.chatgpt },
    },
    system_prompt: DEFAULT_PROMPT,
    default_system_prompt: DEFAULT_PROMPT,
    system_prompt_custom: false,
  };
}

const DEFAULT_PROMPT = "You are the assistant inside TheVault.\n";

/** A status whose chat system prompt is `prompt`, customised. */
function withPrompt(prompt: string): AiStatus {
  return { ...status(), system_prompt: prompt, system_prompt_custom: true };
}

const ANTHROPIC_MODELS = {
  provider: "anthropic",
  model: "claude-opus-5",
  models: [
    { id: "claude-opus-5", display_name: "Claude Opus 5" },
    { id: "claude-sonnet-5", display_name: "Claude Sonnet 5" },
  ],
};

const AUTH_URL = "https://auth.openai.com/api/accounts/authorize?client_id=dynamic_agent_client&state=s1";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  forgetAppInfo();
});

/** Wait for the cards to show. */
async function ready() {
  await screen.findByRole("region", { name: "ChatGPT" });
}

function card(name: string) {
  return within(screen.getByRole("region", { name }));
}

describe("testText", () => {
  it("names the model on success and the kind on failure", () => {
    expect(testText({ ok: true, provider: "anthropic", model: "claude-opus-5", ms: 12, text: "OK", error: null })).toBe(
      "OK from claude-opus-5 in 12 ms",
    );
    expect(
      testText({
        ok: false,
        provider: "openai",
        model: "gpt-6",
        ms: 3,
        text: null,
        error: { kind: "auth", message: "OpenAI refused the key" },
      }),
    ).toBe("Failed (auth): OpenAI refused the key");
  });
});

describe("Settings > AI", () => {
  it("stores a key, shows it as set and never fills it back in", async () => {
    aiStatus.mockResolvedValue(status());
    aiPutKey.mockResolvedValue(status({ anthropic: { configured: true } }, "anthropic"));
    aiModels.mockResolvedValue(ANTHROPIC_MODELS);
    render(() => <Ai />);
    await ready();

    const anthropic = await screen.findByRole("region", { name: "Anthropic" });
    const input = within(anthropic).getByLabelText("API key") as HTMLInputElement;
    expect(input.type).toBe("password");
    expect(input.placeholder).toBe("Paste a key");
    fireEvent.input(input, { target: { value: " sk-ant-secret " } });
    fireEvent.click(within(anthropic).getByRole("button", { name: "Save key" }));

    await waitFor(() => expect(aiPutKey).toHaveBeenCalledWith("anthropic", "sk-ant-secret"));
    expect(await card("Anthropic").findByText("A key is set. Paste another to replace it.")).toBeTruthy();
    const after = card("Anthropic").getByLabelText("API key") as HTMLInputElement;
    expect(after.value).toBe("");
    expect(after.placeholder).toBe("set");
    expect(document.body.textContent).not.toContain("sk-ant-secret");
    await waitFor(() => expect(aiModels).toHaveBeenCalledWith("anthropic"));
  });

  it("removes a key after confirming", async () => {
    aiStatus.mockResolvedValue(status({ openai: { configured: true } }));
    aiModels.mockResolvedValue({ provider: "openai", model: null, models: [{ id: "gpt-6", display_name: null }] });
    aiPutKey.mockResolvedValue(status());
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(() => <Ai />);
    await ready();
    fireEvent.click(await card("OpenAI").findByRole("button", { name: "Remove key" }));
    await waitFor(() => expect(aiPutKey).toHaveBeenCalledWith("openai", ""));
    await waitFor(() => expect(card("OpenAI").queryByRole("button", { name: "Remove key" })).toBeNull());
    confirm.mockRestore();
  });

  it("picks the model from the live list, saves it, and tests it", async () => {
    aiStatus.mockResolvedValue(status({ anthropic: { configured: true } }, "anthropic"));
    aiModels.mockResolvedValue(ANTHROPIC_MODELS);
    aiSaveSettings.mockResolvedValue(status({ anthropic: { configured: true, model: "claude-sonnet-5" } }, "anthropic"));
    aiTest.mockResolvedValue({ ok: true, provider: "anthropic", model: "claude-sonnet-5", ms: 40, text: "OK", error: null });
    render(() => <Ai />);
    await ready();

    const select = (await card("Anthropic").findByLabelText("Model")) as HTMLSelectElement;
    // The preselected model shows before any is saved.
    expect(select.value).toBe("claude-opus-5");
    expect([...select.options].map((o) => o.textContent)).toEqual([
      "Claude Opus 5 (claude-opus-5)",
      "Claude Sonnet 5 (claude-sonnet-5)",
    ]);
    fireEvent.change(select, { target: { value: "claude-sonnet-5" } });
    await waitFor(() => expect(aiSaveSettings).toHaveBeenCalledWith({ models: { anthropic: "claude-sonnet-5" } }));
    await waitFor(() => expect((card("Anthropic").getByLabelText("Model") as HTMLSelectElement).value).toBe("claude-sonnet-5"));

    fireEvent.click(card("Anthropic").getByRole("button", { name: "Test" }));
    await waitFor(() => expect(aiTest).toHaveBeenCalledWith("anthropic", "claude-sonnet-5"));
    expect((await card("Anthropic").findByRole("status")).textContent).toBe("OK from claude-sonnet-5 in 40 ms");
  });

  it("shows a failed Test's kind and message", async () => {
    aiStatus.mockResolvedValue(status({ openai: { configured: true, model: "gpt-6" } }, "openai"));
    aiModels.mockResolvedValue({ provider: "openai", model: "gpt-6", models: [{ id: "gpt-6", display_name: null }] });
    aiTest.mockResolvedValue({
      ok: false,
      provider: "openai",
      model: "gpt-6",
      ms: 5,
      text: null,
      error: { kind: "rate_limit", message: "OpenAI answered 429: Slow down" },
    });
    render(() => <Ai />);
    await ready();
    await card("OpenAI").findByLabelText("Model");
    fireEvent.click(card("OpenAI").getByRole("button", { name: "Test" }));
    expect((await card("OpenAI").findByRole("status")).textContent).toBe(
      "Failed (rate_limit): OpenAI answered 429: Slow down",
    );
  });

  it("makes a provider the default", async () => {
    aiStatus.mockResolvedValue(status({ anthropic: { configured: true }, openai: { configured: true } }, "anthropic"));
    aiModels.mockResolvedValue(ANTHROPIC_MODELS);
    aiSaveSettings.mockResolvedValue(status({ anthropic: { configured: true }, openai: { configured: true } }, "openai"));
    render(() => <Ai />);
    await ready();
    const anthropic = (await card("Anthropic").findByLabelText("Use Anthropic by default")) as HTMLInputElement;
    expect(anthropic.checked).toBe(true);
    fireEvent.click(card("OpenAI").getByLabelText("Use OpenAI by default"));
    await waitFor(() => expect(aiSaveSettings).toHaveBeenCalledWith({ default_provider: "openai" }));
    await waitFor(() => expect((card("OpenAI").getByLabelText("Use OpenAI by default") as HTMLInputElement).checked).toBe(true));
  });

  it("signs in with ChatGPT through the pasted address", async () => {
    aiStatus.mockResolvedValue(status());
    chatgptStart.mockResolvedValue({ auth_url: AUTH_URL });
    chatgptFinish.mockResolvedValue(status({ chatgpt: { configured: true, account: "me@example.com" } }, "chatgpt"));
    aiModels.mockResolvedValue({ provider: "chatgpt", model: null, models: [{ id: "gpt-6.1-sol", display_name: "GPT-6.1 Sol" }] });
    render(() => <Ai />);
    await ready();

    fireEvent.click(await card("ChatGPT").findByRole("button", { name: "Continue with ChatGPT" }));
    await waitFor(() => expect(chatgptStart).toHaveBeenCalled());
    const open = await card("ChatGPT").findByRole("link", { name: "Open the ChatGPT sign-in page" });
    expect(open.getAttribute("href")).toBe(AUTH_URL);
    expect(open.getAttribute("target")).toBe("_blank");
    const pasted = "http://127.0.0.1:53683/auth/callback?code=c1&state=s1&client_id=oaiapp_x";
    fireEvent.input(card("ChatGPT").getByLabelText("Address from the browser"), { target: { value: ` ${pasted} ` } });
    fireEvent.click(card("ChatGPT").getByRole("button", { name: "Finish signing in" }));

    await waitFor(() => expect(chatgptFinish).toHaveBeenCalledWith(pasted));
    expect(await card("ChatGPT").findByText("Using ChatGPT plan as me@example.com")).toBeTruthy();
    expect(card("ChatGPT").getByRole("link", { name: "Manage usage" }).getAttribute("href")).toBe(CHATGPT_USAGE_URL);
    expect(card("ChatGPT").queryByLabelText("Address from the browser")).toBeNull();
    await waitFor(() => expect(aiModels).toHaveBeenCalledWith("chatgpt"));
  });

  it("keeps the paste box and shows the refusal when finishing fails", async () => {
    aiStatus.mockResolvedValue(status());
    chatgptStart.mockResolvedValue({ auth_url: AUTH_URL });
    const { ApiError } = await import("../api");
    chatgptFinish.mockRejectedValue(new ApiError(400, { error: "chatgpt", message: "the ChatGPT ID token carries the wrong nonce" }));
    render(() => <Ai />);
    await ready();
    fireEvent.click(await card("ChatGPT").findByRole("button", { name: "Continue with ChatGPT" }));
    fireEvent.input(await card("ChatGPT").findByLabelText("Address from the browser"), {
      target: { value: "http://127.0.0.1:53683/auth/callback?code=x" },
    });
    fireEvent.click(card("ChatGPT").getByRole("button", { name: "Finish signing in" }));
    expect((await card("ChatGPT").findByRole("alert")).textContent).toContain("wrong nonce");
    expect(card("ChatGPT").getByLabelText("Address from the browser")).toBeTruthy();
  });

  it("disconnects ChatGPT", async () => {
    aiStatus.mockResolvedValue(status({ chatgpt: { configured: true, account: "me@example.com", model: "gpt-6.1-sol" } }));
    aiModels.mockResolvedValue({ provider: "chatgpt", model: "gpt-6.1-sol", models: [{ id: "gpt-6.1-sol", display_name: "GPT-6.1 Sol" }] });
    chatgptDisconnect.mockResolvedValue(status());
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(() => <Ai />);
    await ready();
    fireEvent.click(await card("ChatGPT").findByRole("button", { name: "Disconnect" }));
    await waitFor(() => expect(chatgptDisconnect).toHaveBeenCalled());
    expect(await card("ChatGPT").findByRole("button", { name: "Continue with ChatGPT" })).toBeTruthy();
    expect(screen.queryByText(/Using ChatGPT plan/)).toBeNull();
    confirm.mockRestore();
  });
});

describe("the Chat system prompt card", () => {
  const prompt = () => card("Chat system prompt");
  const box = () => prompt().getByLabelText("System prompt") as HTMLTextAreaElement;
  const button = (name: string) => prompt().getByRole("button", { name }) as HTMLButtonElement;

  it("shows the default prompt in use, with Reset disabled", async () => {
    aiStatus.mockResolvedValue(status());
    render(() => <Ai />);
    await screen.findByRole("region", { name: "Chat system prompt" });
    expect(box().value).toBe(DEFAULT_PROMPT);
    expect(prompt().getByRole("status").textContent).toBe("Default");
    expect(button("Reset to default").disabled).toBe(true);
    expect(button("Save").disabled).toBe(true);
  });

  it("saves an edited prompt and shows it as customised", async () => {
    aiStatus.mockResolvedValue(status());
    aiSaveSettings.mockResolvedValue(withPrompt("Be terse."));
    render(() => <Ai />);
    await screen.findByRole("region", { name: "Chat system prompt" });
    fireEvent.input(box(), { target: { value: "Be terse." } });
    fireEvent.click(button("Save"));
    await waitFor(() => expect(aiSaveSettings).toHaveBeenCalledWith({ system_prompt: "Be terse." }));
    await waitFor(() => expect(prompt().getByRole("status").textContent).toBe("Customised"));
    expect(box().value).toBe("Be terse.");
    expect(button("Reset to default").disabled).toBe(false);
  });

  it("resets a customised prompt to the default", async () => {
    aiStatus.mockResolvedValue(withPrompt("Be terse."));
    aiSaveSettings.mockResolvedValue(status());
    render(() => <Ai />);
    await screen.findByRole("region", { name: "Chat system prompt" });
    expect(box().value).toBe("Be terse.");
    expect(prompt().getByRole("status").textContent).toBe("Customised");
    fireEvent.click(button("Reset to default"));
    await waitFor(() => expect(aiSaveSettings).toHaveBeenCalledWith({ system_prompt: "" }));
    await waitFor(() => expect(box().value).toBe(DEFAULT_PROMPT));
    expect(prompt().getByRole("status").textContent).toBe("Default");
    expect(button("Reset to default").disabled).toBe(true);
  });

  it("refuses a prompt over the limit without sending it", async () => {
    aiStatus.mockResolvedValue(status());
    render(() => <Ai />);
    await screen.findByRole("region", { name: "Chat system prompt" });
    fireEvent.input(box(), { target: { value: "x".repeat(32_001) } });
    fireEvent.click(button("Save"));
    expect((await prompt().findByRole("alert")).textContent).toContain("32000");
    expect(aiSaveSettings).not.toHaveBeenCalled();
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

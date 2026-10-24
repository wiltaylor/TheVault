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
    confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true, agents: true, skills: true },
    web: { search: true, read: true, download: true },
  };
}

const DEFAULT_PROMPT = "You are the assistant inside TheVault.\n";

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
    fireEvent.click(within(anthropic).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(aiPutKey).toHaveBeenCalledWith("anthropic", "sk-ant-secret"));
    expect(await card("Anthropic").findByText("A key is set. Paste another to replace it.")).toBeTruthy();
    const after = card("Anthropic").getByLabelText("API key") as HTMLInputElement;
    expect(after.value).toBe("");
    expect(after.placeholder).toBe("set");
    expect(document.body.textContent).not.toContain("sk-ant-secret");
    await waitFor(() => expect(aiModels).toHaveBeenCalledWith("anthropic"));
  });

  it("saves the API URL and the key with one Save, URL first, and holds Test until they are saved", async () => {
    const url = "https://api.z.ai/api/anthropic";
    aiStatus.mockResolvedValue(status({ anthropic: { configured: true } }, "anthropic"));
    aiModels.mockResolvedValue(ANTHROPIC_MODELS);
    aiSaveSettings.mockResolvedValue(status({ anthropic: { configured: true, url, has_key: true } }, "anthropic"));
    aiPutKey.mockResolvedValue(status({ anthropic: { configured: true, url, has_key: true } }, "anthropic"));
    render(() => <Ai />);
    await ready();

    const test = (await card("Anthropic").findByRole("button", { name: "Test" })) as HTMLButtonElement;
    expect(test.disabled).toBe(false);
    fireEvent.input(card("Anthropic").getByLabelText("API URL"), { target: { value: url } });
    fireEvent.input(card("Anthropic").getByLabelText("API key"), { target: { value: "zai-key" } });
    expect(test.disabled).toBe(true);
    expect(card("Anthropic").getByText(/Save the key and API URL first/)).toBeTruthy();

    fireEvent.click(card("Anthropic").getByRole("button", { name: "Save" }));
    await waitFor(() => expect(aiPutKey).toHaveBeenCalledWith("anthropic", "zai-key"));
    expect(aiSaveSettings).toHaveBeenCalledWith({ urls: { anthropic: url } });
    expect(aiSaveSettings.mock.invocationCallOrder[0]!).toBeLessThan(aiPutKey.mock.invocationCallOrder[0]!);
    expect(await card("Anthropic").findByText(`Saved. Calls go to ${url}.`)).toBeTruthy();
    await waitFor(() => expect(test.disabled).toBe(false));
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

describe("the chat settings moved to Assistant (agents G6)", () => {
  it("links to Assistant's agent screen in place of the system prompt and switch cards", async () => {
    aiStatus.mockResolvedValue(status());
    render(() => <Ai />);
    await ready();
    const link = screen.getByRole("link", { name: "Assistant agent" });
    expect(link.getAttribute("href")).toBe("/agents/assistant");
    expect(link.parentElement).toHaveTextContent("The chat panel's prompt and permissions are on the Assistant agent.");
    expect(screen.queryByRole("region", { name: "Chat system prompt" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Changes the AI makes" })).toBeNull();
    expect(screen.queryByLabelText("System prompt")).toBeNull();
    expect(screen.queryByRole("switch")).toBeNull();
    // Settings > AI keeps the providers.
    expect(screen.getByRole("region", { name: "Anthropic" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "OpenAI" })).toBeTruthy();
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

describe("Settings > AI API URLs (ai D24)", () => {
  it("saves an API URL and clears it back to the official API", async () => {
    aiStatus.mockResolvedValue(status());
    const custom = "https://openrouter.ai/api/v1";
    aiSaveSettings
      .mockResolvedValueOnce(status({ openai: { configured: true, url: custom, has_key: false } }))
      .mockResolvedValueOnce(status());
    aiModels.mockResolvedValue({ provider: "openai", model: null, models: [{ id: "gpt-6", display_name: null }] });
    render(() => <Ai />);
    await ready();

    const input = card("OpenAI").getByLabelText("API URL") as HTMLInputElement;
    expect(input.placeholder).toBe("https://api.openai.com/v1");
    expect((card("Anthropic").getByLabelText("API URL") as HTMLInputElement).placeholder).toBe(
      "https://api.anthropic.com/v1",
    );
    expect(card("OpenAI").getByText(/Leave the URL empty for the official API/)).toBeTruthy();
    const save = card("OpenAI").getByRole("button", { name: "Save" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    fireEvent.input(input, { target: { value: ` ${custom} ` } });
    fireEvent.click(save);
    await waitFor(() => expect(aiSaveSettings).toHaveBeenCalledWith({ urls: { openai: custom } }));
    expect(await card("OpenAI").findByText(`Saved. Calls go to ${custom}.`)).toBeTruthy();
    // Configured with no key: the key is optional and not shown as set.
    const key = card("OpenAI").getByLabelText("API key") as HTMLInputElement;
    expect(key.placeholder).toBe("Optional with a custom API URL");
    expect(card("OpenAI").queryByRole("button", { name: "Remove key" })).toBeNull();
    await waitFor(() => expect(aiModels).toHaveBeenCalledWith("openai"));

    const after = card("OpenAI").getByLabelText("API URL") as HTMLInputElement;
    expect(after.value).toBe(custom);
    fireEvent.input(after, { target: { value: "" } });
    fireEvent.click(card("OpenAI").getByRole("button", { name: "Save" }));
    await waitFor(() => expect(aiSaveSettings).toHaveBeenLastCalledWith({ urls: { openai: "" } }));
    expect(await card("OpenAI").findByText("Saved. Calls go to the official OpenAI API.")).toBeTruthy();
    expect((card("OpenAI").getByLabelText("API URL") as HTMLInputElement).value).toBe("");
  });

  it("shows a refused URL's message", async () => {
    aiStatus.mockResolvedValue(status());
    aiSaveSettings.mockRejectedValue(new Error("The API URL must start with http:// or https://."));
    render(() => <Ai />);
    await ready();
    fireEvent.input(card("Anthropic").getByLabelText("API URL"), { target: { value: "ftp://example.com" } });
    fireEvent.click(card("Anthropic").getByRole("button", { name: "Save" }));
    expect((await card("Anthropic").findByRole("alert")).textContent).toContain("must start with http://");
  });

  it("types the model when a custom URL lists none", async () => {
    const url = "http://10.0.0.5:11434";
    aiStatus.mockResolvedValue(status({ openai: { configured: true, url, has_key: false } }));
    aiModels.mockResolvedValue({ provider: "openai", model: null, models: [] });
    aiSaveSettings.mockResolvedValue(status({ openai: { configured: true, url, has_key: false, model: "llama-4" } }));
    render(() => <Ai />);
    await ready();

    const model = (await card("OpenAI").findByLabelText("Model")) as HTMLInputElement;
    expect(model.tagName).toBe("INPUT");
    expect(card("OpenAI").getByText(/lists no models/)).toBeTruthy();
    fireEvent.input(model, { target: { value: " llama-4 " } });
    fireEvent.change(model, { target: { value: " llama-4 " } });
    await waitFor(() => expect(aiSaveSettings).toHaveBeenCalledWith({ models: { openai: "llama-4" } }));
    await waitFor(() => expect((card("OpenAI").getByLabelText("Model") as HTMLInputElement).value).toBe("llama-4"));
  });

  it("types the model when a custom URL's model list fails, and keeps the select otherwise", async () => {
    aiStatus.mockResolvedValue(
      status({
        anthropic: { configured: true, url: "https://llm.example/v1", has_key: true, model: "my-model" },
        openai: { configured: true, has_key: true },
      }),
    );
    aiModels.mockImplementation(async (p: string) => {
      if (p === "anthropic") throw new Error("Anthropic answered 404: Not found");
      throw new Error("OpenAI answered 500: down");
    });
    render(() => <Ai />);
    await ready();

    const model = (await card("Anthropic").findByLabelText("Model")) as HTMLInputElement;
    expect(model.tagName).toBe("INPUT");
    await waitFor(() => expect((card("Anthropic").getByLabelText("Model") as HTMLInputElement).value).toBe("my-model"));
    expect(card("Anthropic").getByText(/could not be read/)).toBeTruthy();
    expect(card("Anthropic").getByRole("alert").textContent).toContain("404");
    // A key is set too: it shows, and Remove key is offered.
    expect(card("Anthropic").getByRole("button", { name: "Remove key" })).toBeTruthy();
    // The official OpenAI API keeps the error and no model field to type in.
    await waitFor(() => expect(card("OpenAI").getByRole("alert").textContent).toContain("500"));
    expect(card("OpenAI").queryByRole("textbox", { name: "Model" })).toBeNull();
  });
});

describe("Settings > AI API type (ai D25)", () => {
  const url = "https://api.z.ai/api/coding/paas/v4";
  const MODELS = { provider: "openai", model: "glm-5.3-flash", models: [{ id: "glm-5.3-flash", display_name: null }] };

  it("names the API a Test went over", () => {
    expect(
      testText({
        ok: true,
        provider: "openai",
        model: "glm-5.3-flash",
        ms: 30,
        text: "OK",
        api: "chat_completions",
        error: null,
      }),
    ).toBe("OK from glm-5.3-flash in 30 ms (Chat Completions API)");
    expect(
      testText({ ok: true, provider: "openai", model: "gpt-6", ms: 9, text: "OK", api: null, error: null }),
    ).toBe("OK from gpt-6 in 9 ms");
  });

  it("shows the select only with a custom URL on the OpenAI card, and saves it with the card", async () => {
    aiStatus.mockResolvedValue(status({ openai: { configured: true, has_key: true, api: "responses" } }));
    aiModels.mockResolvedValue(MODELS);
    aiSaveSettings.mockResolvedValue(
      status({ openai: { configured: true, has_key: true, url, api: "chat_completions" } }),
    );
    render(() => <Ai />);
    await ready();

    expect(card("OpenAI").queryByLabelText("API type")).toBeNull();
    fireEvent.input(card("OpenAI").getByLabelText("API URL"), { target: { value: url } });
    const select = card("OpenAI").getByLabelText("API type") as HTMLSelectElement;
    expect([...select.options].map((o) => o.textContent)).toEqual(["Responses API", "Chat Completions API"]);
    expect(select.value).toBe("responses");
    fireEvent.change(select, { target: { value: "chat_completions" } });
    fireEvent.input(card("Anthropic").getByLabelText("API URL"), { target: { value: "https://x.example" } });
    expect(card("Anthropic").queryByLabelText("API type")).toBeNull();

    fireEvent.click(card("OpenAI").getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(aiSaveSettings).toHaveBeenCalledWith({ urls: { openai: url }, openai_api: "chat_completions" }),
    );
    expect(aiSaveSettings).toHaveBeenCalledTimes(1);
    expect(aiPutKey).not.toHaveBeenCalled();
    expect(await card("OpenAI").findByText(`Saved. Calls go to ${url}.`)).toBeTruthy();
    expect((card("OpenAI").getByLabelText("API type") as HTMLSelectElement).value).toBe("chat_completions");

    // The type alone is a change to save, and holds Test until it is saved.
    const test = card("OpenAI").getByRole("button", { name: "Test" }) as HTMLButtonElement;
    expect(test.disabled).toBe(false);
    fireEvent.change(card("OpenAI").getByLabelText("API type"), { target: { value: "responses" } });
    expect(test.disabled).toBe(true);
    fireEvent.click(card("OpenAI").getByRole("button", { name: "Save" }));
    await waitFor(() => expect(aiSaveSettings).toHaveBeenLastCalledWith({ openai_api: "responses" }));
  });

  it("shows the type a Test detected and the saved type after it", async () => {
    aiStatus
      .mockResolvedValueOnce(status({ openai: { configured: true, url, model: "glm-5.3-flash", api: "responses" } }))
      .mockResolvedValue(status({ openai: { configured: true, url, model: "glm-5.3-flash", api: "chat_completions" } }));
    aiModels.mockResolvedValue(MODELS);
    aiTest.mockResolvedValue({
      ok: true,
      provider: "openai",
      model: "glm-5.3-flash",
      ms: 120,
      text: "OK",
      api: "chat_completions",
      error: null,
    });
    render(() => <Ai />);
    await ready();

    expect((card("OpenAI").getByLabelText("API type") as HTMLSelectElement).value).toBe("responses");
    await card("OpenAI").findByLabelText("Model");
    fireEvent.click(card("OpenAI").getByRole("button", { name: "Test" }));
    await waitFor(() => expect(aiTest).toHaveBeenCalledWith("openai", "glm-5.3-flash"));
    expect((await card("OpenAI").findByRole("status")).textContent).toBe(
      "OK from glm-5.3-flash in 120 ms (Chat Completions API)",
    );
    await waitFor(() =>
      expect((card("OpenAI").getByLabelText("API type") as HTMLSelectElement).value).toBe("chat_completions"),
    );
    expect(aiStatus).toHaveBeenCalledTimes(2);
    expect((card("OpenAI").getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
  });
});

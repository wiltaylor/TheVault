import { For, type JSX, Show, createEffect, createMemo, createSignal, on } from "solid-js";
import { ASSISTANT } from "../agents";
import {
  type AiContext,
  type AiModel,
  type AiProvider,
  type AiStatus,
  type AiTestResult,
  type OpenAiApi,
  api,
} from "../api";
import { Spinner } from "../components/SyncStatus";
import { errorText } from "./TypeEditor";

// Settings > AI (ai D6, D7, D8, D16): one card per provider. The Anthropic and OpenAI cards take
// an API key, which the server stores and never answers back, and an API URL for another service
// that speaks the same API, which makes the key optional (ai D24), and the OpenAI card with such a
// URL an API type, Responses or Chat Completions, which Test detects (ai D25); the ChatGPT card signs in with
// OpenAI's Sign in with ChatGPT, through the same paste-the-address step as Google's card. Each
// card lists the provider's models live, saves the chosen default model, tests it, and can be
// made the default provider. The chat's system prompt and the switches that let it make some kinds
// of change without asking are the built-in Assistant agent's (agents G6), so a line links to its
// agent screen. The section is web-app only: the desktop and Android apps talk to their embedded
// server, which holds no AI credentials.

/** Where a ChatGPT plan's usage is managed. */
export const CHATGPT_USAGE_URL = "https://chatgpt.com/settings/usage";

/** The loopback address Sign in with ChatGPT sends the browser to, where nothing listens. */
export const CHATGPT_REDIRECT = "http://127.0.0.1:53683/auth/callback";

/** The spinner a button shows while its request is in flight. */
function Busy(props: { on: boolean }) {
  return (
    <Show when={props.on}>
      <Spinner />{" "}
    </Show>
  );
}

/** The OpenAI API types, as the API type select names them (ai D25). */
export const OPENAI_APIS: Record<OpenAiApi, string> = {
  responses: "Responses API",
  chat_completions: "Chat Completions API",
};

/** How a Test went, in a line, naming the OpenAI API it went over when there is a choice. */
export function testText(r: AiTestResult): string {
  if (r.ok) {
    const over = r.api ? ` (${OPENAI_APIS[r.api]})` : "";
    return `OK from ${r.model ?? "the model"} in ${r.ms} ms${over}`;
  }
  const e = r.error;
  return e ? `Failed (${e.kind}): ${e.message}` : "Failed";
}

export function modelLabel(m: AiModel): string {
  return m.display_name && m.display_name !== m.id ? `${m.display_name} (${m.id})` : m.id;
}

/**
 * What every provider card has below its credentials: the default model, chosen from the live
 * list, the Test button and its result, and the default-provider choice.
 */
function ModelAndTest(props: {
  provider: AiProvider;
  label: string;
  status: AiStatus;
  onStatus: (s: AiStatus) => void;
  /** The card holds a key or API URL not saved yet: a Test now would use the old ones. */
  unsaved?: boolean;
}) {
  const p = () => props.status.providers[props.provider];
  const [models, setModels] = createSignal<AiModel[] | null>(null);
  const [preselected, setPreselected] = createSignal<string | null>(null);
  const [listError, setListError] = createSignal<string | null>(null);
  const [result, setResult] = createSignal<AiTestResult | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<"test" | "model" | "default" | null>(null);

  async function loadModels() {
    setListError(null);
    try {
      const list = await api.aiModels(props.provider);
      setModels(list.models);
      setPreselected(list.model);
    } catch (e) {
      setModels(null);
      setListError(errorText(e));
    }
  }
  // List the models once the provider is set up, and again after a new key, sign-in or API URL.
  const configured = createMemo(() => p().configured);
  const url = createMemo(() => p().url ?? "");
  createEffect(
    on(
      [configured, url],
      ([configured]) => {
        setResult(null);
        if (configured) void loadModels();
        else setModels(null);
      },
    ),
  );

  const chosen = () => p().model ?? preselected() ?? "";
  // On a custom API URL whose model list fails or is empty, the model is typed (ai D24).
  const typed = () => !!p().url && (listError() !== null || models()?.length === 0);
  const [typedModel, setTypedModel] = createSignal("");
  createEffect(on(chosen, (m) => setTypedModel(m)));

  async function act(what: "test" | "model" | "default", f: () => Promise<void>) {
    setError(null);
    setPending(what);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPending(null);
    }
  }

  function pick(model: string) {
    setResult(null);
    void act("model", async () => props.onStatus(await api.aiSaveSettings({ models: { [props.provider]: model } })));
  }

  function test() {
    setResult(null);
    void act("test", async () => {
      const r = await api.aiTest(props.provider, chosen() || undefined);
      setResult(r);
      // A Test with no model chosen saves the preselected one, and one that detected another
      // API type saves that (ai D25).
      if (!p().model || (r.api && r.api !== (p().api ?? "responses"))) props.onStatus(await api.aiStatus());
    });
  }

  function makeDefault() {
    void act("default", async () => props.onStatus(await api.aiSaveSettings({ default_provider: props.provider })));
  }

  const id = (what: string) => `ai-${props.provider}-${what}`;

  return (
    <Show when={p().configured}>
      <div class="form-grid">
        <label for={id("model")}>Model</label>
        <Show
          when={!typed()}
          fallback={
            <>
              <input
                id={id("model")}
                list={id("model-ids")}
                autocomplete="off"
                spellcheck={false}
                placeholder="Type a model id"
                value={typedModel()}
                disabled={pending() !== null}
                onInput={(e) => setTypedModel(e.currentTarget.value)}
                onChange={(e) => {
                  const m = e.currentTarget.value.trim();
                  if (m && m !== p().model) pick(m);
                }}
              />
              <datalist id={id("model-ids")}>
                <For each={[...new Set([...(models() ?? []).map((m) => m.id), chosen()].filter((m) => m))]}>
                  {(m) => <option value={m} />}
                </For>
              </datalist>
            </>
          }
        >
          <Show
            when={models()}
            fallback={<span class="muted">{listError() ?? "Listing the models…"}</span>}
          >
            {(list) => (
              <select
                id={id("model")}
                value={chosen()}
                disabled={pending() !== null}
                onChange={(e) => pick(e.currentTarget.value)}
              >
                <Show when={!chosen()}>
                  <option value="">Choose a model</option>
                </Show>
                <For each={list()}>{(m) => <option value={m.id}>{modelLabel(m)}</option>}</For>
              </select>
            )}
          </Show>
        </Show>
      </div>
      <Show when={typed()}>
        <p class="muted">
          {listError() === null
            ? "This service lists no models: type the id of the one to use."
            : "The model list could not be read: type the id of the model to use."}
        </p>
      </Show>
      <Show when={listError()}>
        <p class="error" role="alert">
          {listError()}
        </p>
      </Show>
      <div class="actions">
        <button
          type="button"
          disabled={pending() !== null || props.unsaved}
          title={props.unsaved ? "Save the key and API URL first" : undefined}
          aria-busy={pending() === "test" ? "true" : undefined}
          onClick={test}
        >
          <Busy on={pending() === "test"} />
          Test
        </button>
        <label class="ai-default">
          <input
            type="radio"
            name="ai-default-provider"
            checked={props.status.default_provider === props.provider}
            disabled={pending() !== null}
            onChange={makeDefault}
          />{" "}
          Use {props.label} by default
        </label>
      </div>
      <Show when={props.unsaved}>
        <p class="muted">Save the key and API URL first: Test uses the saved ones.</p>
      </Show>
      <Show when={result()}>
        {(r) => (
          <p class={r().ok ? "ai-test-ok" : "error"} role="status">
            {testText(r())}
          </p>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </Show>
  );
}

function Card(props: { title: string; blurb: JSX.Element; children: JSX.Element }) {
  return (
    <section class="field-card ai-card" aria-label={props.title}>
      <header>
        <h2>{props.title}</h2>
        <p class="muted">{props.blurb}</p>
      </header>
      {props.children}
    </section>
  );
}

/** The official API URLs, the API URL fields' placeholders (ai D24). */
export const OFFICIAL_URLS = {
  anthropic: "https://api.anthropic.com/v1",
  openai: "https://api.openai.com/v1",
} as const;

/**
 * An Anthropic or OpenAI card (ai D24): the API key, stored and never shown again, and the API
 * URL, empty for the official API or another service that speaks the same API, saved together
 * by one Save. With a custom URL the key is optional.
 */
export function KeyCard(props: {
  provider: "anthropic" | "openai";
  label: string;
  blurb: string;
  status: AiStatus;
  onStatus: (s: AiStatus) => void;
}) {
  const p = () => props.status.providers[props.provider];
  const storedUrl = () => p().url ?? "";
  const storedApi = (): OpenAiApi => p().api ?? "responses";
  const [key, setKey] = createSignal("");
  const [url, setUrl] = createSignal(storedUrl());
  const [apiType, setApiType] = createSignal<OpenAiApi>(storedApi());
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<"save" | "remove" | null>(null);
  createEffect(on(storedUrl, (u) => setUrl(u), { defer: true }));
  createEffect(on(storedApi, (a) => setApiType(a), { defer: true }));
  // The API type is the OpenAI card's, and only with a custom URL (ai D25).
  const typed = () => props.provider === "openai" && url().trim() !== "";
  // A custom API URL makes the provider configured with no key (ai D24), so `has_key` says.
  const set = () => p().has_key ?? p().configured;
  const urlChanged = () => url().trim() !== storedUrl();
  const apiChanged = () => typed() && apiType() !== storedApi();
  const unsaved = () => urlChanged() || apiChanged() || key().trim() !== "";
  const id = (what: string) => `ai-${props.provider}-${what}`;

  async function act(what: "save" | "remove", f: () => Promise<string | null>) {
    setError(null);
    setNotice(null);
    setPending(what);
    try {
      setNotice(await f());
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPending(null);
    }
  }

  function save(ev: SubmitEvent) {
    ev.preventDefault();
    const k = key().trim();
    const u = url().trim();
    if (!k && !urlChanged() && !apiChanged()) {
      setError(set() || u ? "Nothing to save: paste a new key or change the API URL." : `Paste the ${props.label} API key first.`);
      return;
    }
    void act("save", async () => {
      // The URL and API type first: a key saved for a new service then belongs with it.
      if (urlChanged() || apiChanged()) {
        props.onStatus(
          await api.aiSaveSettings({
            ...(urlChanged() ? { urls: { [props.provider]: u } } : {}),
            ...(apiChanged() ? { openai_api: apiType() } : {}),
          }),
        );
      }
      if (k) {
        props.onStatus(await api.aiPutKey(props.provider, k));
        setKey("");
      }
      return u ? `Saved. Calls go to ${u}.` : `Saved. Calls go to the official ${props.label} API.`;
    });
  }

  function remove() {
    if (!confirm(`Remove the ${props.label} API key? TheVault forgets it.`)) return;
    void act("remove", async () => {
      props.onStatus(await api.aiPutKey(props.provider, ""));
      setKey("");
      return null;
    });
  }

  return (
    <Card title={props.label} blurb={props.blurb}>
      <form class="admin-form" aria-label={`${props.label} connection`} onSubmit={save}>
        <div class="form-grid">
          <label for={id("url")}>API URL</label>
          <input
            id={id("url")}
            type="url"
            autocomplete="off"
            spellcheck={false}
            placeholder={OFFICIAL_URLS[props.provider]}
            value={url()}
            onInput={(e) => setUrl(e.currentTarget.value)}
          />
          <Show when={typed()}>
            <label for={id("api")}>API type</label>
            <select id={id("api")} value={apiType()} onChange={(e) => setApiType(e.currentTarget.value as OpenAiApi)}>
              <For each={Object.entries(OPENAI_APIS)}>{([value, label]) => <option value={value}>{label}</option>}</For>
            </select>
          </Show>
          <label for={id("key")}>API key</label>
          <input
            id={id("key")}
            type="password"
            autocomplete="off"
            placeholder={set() ? "set" : url().trim() ? "Optional with a custom API URL" : "Paste a key"}
            value={key()}
            onInput={(e) => setKey(e.currentTarget.value)}
          />
        </div>
        <p class="muted">
          Leave the URL empty for the official API. Another service must speak the{" "}
          {props.provider === "anthropic"
            ? "Anthropic Messages API"
            : "OpenAI Responses API or Chat Completions API (Test finds which)"}
          ; with its own URL the key is optional.
        </p>
        <Show when={set()}>
          <p class="muted">A key is set. Paste another to replace it.</p>
        </Show>
        <div class="actions">
          <button
            type="submit"
            class="primary"
            disabled={pending() !== null || !unsaved()}
            aria-busy={pending() === "save" ? "true" : undefined}
          >
            <Busy on={pending() === "save"} />
            Save
          </button>
          <Show when={set()}>
            <button
              type="button"
              class="danger"
              disabled={pending() !== null}
              aria-busy={pending() === "remove" ? "true" : undefined}
              onClick={remove}
            >
              <Busy on={pending() === "remove"} />
              Remove key
            </button>
          </Show>
        </div>
      </form>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={notice()}>
        <p class="muted">{notice()}</p>
      </Show>
      <ModelAndTest
        provider={props.provider}
        label={props.label}
        status={props.status}
        onStatus={props.onStatus}
        unsaved={unsaved()}
      />
    </Card>
  );
}

/** The ChatGPT card: Sign in with ChatGPT, through a pasted address, and the plan's account. */
export function ChatGptCard(props: { status: AiStatus; onStatus: (s: AiStatus) => void }) {
  const [authUrl, setAuthUrl] = createSignal<string | null>(null);
  const [pasted, setPasted] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<"start" | "finish" | "disconnect" | null>(null);
  const busy = () => pending() !== null;
  const doing = (a: "start" | "finish" | "disconnect") => pending() === a;
  const chatgpt = () => props.status.providers.chatgpt;

  async function act(a: "start" | "finish" | "disconnect", f: () => Promise<void>) {
    setError(null);
    setNotice(null);
    setPending(a);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPending(null);
    }
  }

  function start() {
    void act("start", async () => {
      const { auth_url } = await api.chatgptStart();
      setAuthUrl(auth_url);
      setPasted("");
    });
  }

  function finish(ev: SubmitEvent) {
    ev.preventDefault();
    void act("finish", async () => {
      props.onStatus(await api.chatgptFinish(pasted().trim()));
      setAuthUrl(null);
      setPasted("");
    });
  }

  function disconnect() {
    if (!confirm("Disconnect ChatGPT? TheVault signs out of your plan and forgets the sign-in.")) return;
    void act("disconnect", async () => {
      props.onStatus(await api.chatgptDisconnect());
      setNotice("Disconnected from ChatGPT.");
    });
  }

  return (
    <Card title="ChatGPT" blurb="Your ChatGPT plan, through OpenAI's Sign in with ChatGPT.">
      <Show
        when={chatgpt().configured}
        fallback={
          <Show
            when={authUrl()}
            fallback={
              <button
                type="button"
                class="primary"
                disabled={busy()}
                aria-busy={doing("start") ? "true" : undefined}
                onClick={start}
              >
                <Busy on={doing("start")} />
                Continue with ChatGPT
              </button>
            }
          >
            {(url) => (
              <form class="admin-form" aria-label="Finish signing in with ChatGPT" onSubmit={finish}>
                <ol class="google-steps">
                  <li>
                    <a href={url()} target="_blank" rel="noopener noreferrer">
                      Open the ChatGPT sign-in page
                    </a>{" "}
                    and allow TheVault to use your plan.
                  </li>
                  <li>
                    The browser then fails to load a <code>127.0.0.1:53683</code> page. That is expected: copy the whole
                    address from its address bar.
                  </li>
                  <li>Paste it here within 10 minutes.</li>
                </ol>
                <label for="chatgpt-pasted">Address from the browser</label>
                <input
                  id="chatgpt-pasted"
                  autocomplete="off"
                  placeholder={`${CHATGPT_REDIRECT}?code=…&state=…`}
                  value={pasted()}
                  onInput={(e) => setPasted(e.currentTarget.value)}
                />
                <div class="actions">
                  <button
                    type="submit"
                    class="primary"
                    disabled={busy() || !pasted().trim()}
                    aria-busy={doing("finish") ? "true" : undefined}
                  >
                    <Busy on={doing("finish")} />
                    Finish signing in
                  </button>
                  <button type="button" disabled={doing("finish")} onClick={() => setAuthUrl(null)}>
                    Cancel
                  </button>
                </div>
              </form>
            )}
          </Show>
        }
      >
        <p class="google-account" role="status">
          Using ChatGPT plan as {chatgpt().account ?? "an unknown account"}
        </p>
        <p>
          <a href={CHATGPT_USAGE_URL} target="_blank" rel="noopener noreferrer">
            Manage usage
          </a>
        </p>
      </Show>
      <ModelAndTest provider="chatgpt" label="ChatGPT" status={props.status} onStatus={props.onStatus} />
      <Show when={chatgpt().configured}>
        <button
          type="button"
          class="danger"
          disabled={busy()}
          aria-busy={doing("disconnect") ? "true" : undefined}
          onClick={disconnect}
        >
          <Busy on={doing("disconnect")} />
          Disconnect
        </button>
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={notice()}>
        <p class="muted">{notice()}</p>
      </Show>
    </Card>
  );
}

/** Settings > AI. */
const CONTEXT_LABELS: Record<AiProvider, string> = { anthropic: "Anthropic", openai: "OpenAI", chatgpt: "ChatGPT plan" };

/** The context windows TheVault plans with, and when a long chat is compacted. */
export function ContextCard(props: { status: AiStatus & { context: AiContext }; onStatus: (s: AiStatus) => void }) {
  const c = () => props.status.context;
  const [error, setError] = createSignal<string | null>(null);
  const [saved, setSaved] = createSignal<string | null>(null);
  async function save(body: Parameters<typeof api.aiSaveSettings>[0]["context"], what: string) {
    setError(null);
    setSaved(null);
    try {
      props.onStatus(await api.aiSaveSettings({ context: body }));
      setSaved(what);
    } catch (e) {
      setError(errorText(e));
    }
  }
  return (
    <Card
      title="Context"
      blurb={
        <>
          How many tokens each provider's models take, and when a long chat is summarized for the model. Type{" "}
          <code>/compact</code> in a chat to compact it at once.
        </>
      }
    >
      <div class="ai-context">
        <For each={Object.keys(CONTEXT_LABELS) as AiProvider[]}>
          {(p) => (
            <label>
              {CONTEXT_LABELS[p]} window, tokens
              <input
                type="number"
                min="4000"
                max="10000000"
                step="1000"
                value={c().windows[p]?.window ?? ""}
                onChange={(e) => {
                  const n = e.currentTarget.valueAsNumber;
                  void save({ windows: { [p]: Number.isFinite(n) ? n : null } }, `${CONTEXT_LABELS[p]} window saved.`);
                }}
              />
              <Show when={c().windows[p]?.set}>
                <button type="button" class="link" onClick={() => void save({ windows: { [p]: null } }, `${CONTEXT_LABELS[p]} window reset.`)}>
                  Use the default
                </button>
              </Show>
            </label>
          )}
        </For>
        <label class="ai-context-switch">
          <input
            type="checkbox"
            checked={c().auto_compact}
            onChange={(e) => void save({ auto_compact: e.currentTarget.checked }, "Saved.")}
          />
          Compact long chats on their own
        </label>
        <label>
          Compact at, percent of the window
          <input
            type="number"
            min="50"
            max="95"
            value={c().compact_at}
            disabled={!c().auto_compact}
            onChange={(e) => void save({ compact_at: e.currentTarget.valueAsNumber }, "Saved.")}
          />
        </label>
      </div>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={saved()}>
        <p class="muted" role="status">
          {saved()}
        </p>
      </Show>
    </Card>
  );
}

export function Ai() {
  const [status, setStatus] = createSignal<AiStatus | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  void (async () => {
    try {
      setStatus(await api.aiStatus());
    } catch (e) {
      setError(errorText(e));
    }
  })();
  return (
    <section class="list-page">
      <header class="note-header">
        <h1>AI</h1>
      </header>
      <p class="muted">
        Scripts and the AI panel call these providers with your own credentials. The server keeps them encrypted and
        never shows them again.
      </p>
      <p class="ai-agent-link">
        The chat panel's prompt and permissions are on the{" "}
        <a href={`/agents/${ASSISTANT}`}>Assistant agent</a>.
      </p>
      <Show when={status()}>
        {(s) => (
          <>
            <Show when={!s().secrets_key}>
              <p class="error" role="alert">
                This server has no secrets key, so it cannot store AI credentials.
              </p>
            </Show>
            <KeyCard
              provider="anthropic"
              label="Anthropic"
              blurb="Claude, with an API key from console.anthropic.com."
              status={s()}
              onStatus={setStatus}
            />
            <KeyCard
              provider="openai"
              label="OpenAI"
              blurb="OpenAI's models, with an API key from platform.openai.com."
              status={s()}
              onStatus={setStatus}
            />
            <ChatGptCard status={s()} onStatus={setStatus} />
            <Show when={s().context ? (s() as AiStatus & { context: AiContext }) : undefined}>
              {(withContext) => <ContextCard status={withContext()} onStatus={setStatus} />}
            </Show>
          </>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </section>
  );
}

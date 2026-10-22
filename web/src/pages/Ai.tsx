import { For, type JSX, Show, createEffect, createSignal, on } from "solid-js";
import { type AiModel, type AiProvider, type AiStatus, type AiTestResult, api } from "../api";
import { Spinner } from "../components/SyncStatus";
import { errorText } from "./TypeEditor";

// Settings > AI (ai D6, D7, D8, D16): one card per provider. The Anthropic and OpenAI cards take
// an API key, which the server stores and never answers back; the ChatGPT card signs in with
// OpenAI's Sign in with ChatGPT, through the same paste-the-address step as Google's card. Each
// card lists the provider's models live, saves the chosen default model, tests it, and can be
// made the default provider. The section is web-app only: the desktop and Android apps talk to
// their embedded server, which holds no AI credentials.

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

/** How a Test went, in a line. */
export function testText(r: AiTestResult): string {
  if (r.ok) return `OK from ${r.model ?? "the model"} in ${r.ms} ms`;
  const e = r.error;
  return e ? `Failed (${e.kind}): ${e.message}` : "Failed";
}

function modelLabel(m: AiModel): string {
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
  // List the models once the provider is set up, and again after a new key or sign-in.
  createEffect(
    on(
      () => p().configured,
      (configured) => {
        setResult(null);
        if (configured) void loadModels();
        else setModels(null);
      },
    ),
  );

  const chosen = () => p().model ?? preselected() ?? "";

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
      setResult(await api.aiTest(props.provider, chosen() || undefined));
      // A Test with no model chosen saves the preselected one.
      if (!p().model) props.onStatus(await api.aiStatus());
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
      </div>
      <Show when={listError()}>
        <p class="error" role="alert">
          {listError()}
        </p>
      </Show>
      <div class="actions">
        <button
          type="button"
          disabled={pending() !== null}
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

/** An Anthropic or OpenAI card: the API key, stored and never shown again. */
export function KeyCard(props: {
  provider: "anthropic" | "openai";
  label: string;
  blurb: string;
  status: AiStatus;
  onStatus: (s: AiStatus) => void;
}) {
  const [key, setKey] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<"save" | "remove" | null>(null);
  const set = () => props.status.providers[props.provider].configured;
  const id = `ai-${props.provider}-key`;

  async function act(what: "save" | "remove", value: string) {
    setError(null);
    setPending(what);
    try {
      props.onStatus(await api.aiPutKey(props.provider, value));
      setKey("");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPending(null);
    }
  }

  function save(ev: SubmitEvent) {
    ev.preventDefault();
    if (!key().trim()) {
      setError(`Paste the ${props.label} API key first.`);
      return;
    }
    void act("save", key().trim());
  }

  function remove() {
    if (!confirm(`Remove the ${props.label} API key? TheVault forgets it.`)) return;
    void act("remove", "");
  }

  return (
    <Card title={props.label} blurb={props.blurb}>
      <form class="admin-form" aria-label={`${props.label} API key`} onSubmit={save}>
        <div class="form-grid">
          <label for={id}>API key</label>
          <input
            id={id}
            type="password"
            autocomplete="off"
            placeholder={set() ? "set" : "Paste a key"}
            value={key()}
            onInput={(e) => setKey(e.currentTarget.value)}
          />
        </div>
        <Show when={set()}>
          <p class="muted">A key is set. Paste another to replace it.</p>
        </Show>
        <div class="actions">
          <button
            type="submit"
            class="primary"
            disabled={pending() !== null}
            aria-busy={pending() === "save" ? "true" : undefined}
          >
            <Busy on={pending() === "save"} />
            Save key
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
      <ModelAndTest provider={props.provider} label={props.label} status={props.status} onStatus={props.onStatus} />
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

/** The most characters a chat system prompt may have (ai D17); the server holds the same limit. */
export const SYSTEM_PROMPT_MAX = 32_000;

/**
 * The chat panel's system prompt (ai D17): the text in use, editable, with Save and Reset to
 * default. The open note's id, type and title are added after it by the server, and the tools
 * reach the model through their definitions, so neither is part of this text.
 */
export function SystemPromptCard(props: { status: AiStatus; onStatus: (s: AiStatus) => void }) {
  const [text, setText] = createSignal(props.status.system_prompt);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<"save" | "reset" | null>(null);
  // Follow the prompt in use whenever the server answers a new one.
  createEffect(on(() => props.status.system_prompt, (p) => setText(p), { defer: true }));
  const custom = () => props.status.system_prompt_custom;
  const changed = () => text() !== props.status.system_prompt;
  const length = () => [...text()].length;

  async function act(what: "save" | "reset", value: string, done: string) {
    setError(null);
    setNotice(null);
    setPending(what);
    try {
      props.onStatus(await api.aiSaveSettings({ system_prompt: value }));
      setNotice(done);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPending(null);
    }
  }

  function save(ev: SubmitEvent) {
    ev.preventDefault();
    if (length() > SYSTEM_PROMPT_MAX) {
      setError(`The prompt is ${length()} characters; the most it can be is ${SYSTEM_PROMPT_MAX}.`);
      return;
    }
    // The default text saved as it is, or an empty box, means the default.
    const value = text() === props.status.default_system_prompt ? "" : text();
    void act("save", value, "Saved. The next chat message uses it.");
  }

  function reset() {
    void act("reset", "", "Reset to the default prompt.");
  }

  return (
    <Card
      title="Chat system prompt"
      blurb="What the AI panel tells the model before every conversation. The open note and the tool list are added for you."
    >
      <form class="admin-form" aria-label="Chat system prompt" onSubmit={save}>
        <p class="muted" role="status">
          {custom() ? "Customised" : "Default"}
        </p>
        <label for="ai-system-prompt">System prompt</label>
        <textarea
          id="ai-system-prompt"
          class="ai-system-prompt"
          rows={16}
          spellcheck={false}
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
        />
        <p class={length() > SYSTEM_PROMPT_MAX ? "error" : "muted"}>
          {length()} of {SYSTEM_PROMPT_MAX} characters
        </p>
        <div class="actions">
          <button
            type="submit"
            class="primary"
            disabled={pending() !== null || !changed()}
            aria-busy={pending() === "save" ? "true" : undefined}
          >
            <Busy on={pending() === "save"} />
            Save
          </button>
          <button
            type="button"
            disabled={pending() !== null || !custom()}
            aria-busy={pending() === "reset" ? "true" : undefined}
            onClick={reset}
          >
            <Busy on={pending() === "reset"} />
            Reset to default
          </button>
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
    </Card>
  );
}

/** Settings > AI. */
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
            <SystemPromptCard status={s()} onStatus={setStatus} />
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

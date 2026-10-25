import { For, type JSX, Show, createEffect, createMemo, createSignal, on } from "solid-js";
import { ASSISTANT } from "../agents";
import {
  type AiAccountStatus,
  type AiContext,
  type AiKind,
  type AiModel,
  type AiStatus,
  type AiTestResult,
  type AiUsage,
  api,
} from "../api";
import { Spinner } from "../components/SyncStatus";
import { errorText } from "./TypeEditor";

// Settings > AI (accounts A11): a table of named AI accounts, each of one kind (accounts A1) that
// the server lists with its label, default URL and default window. Add picks a kind first, then
// shows only that kind's fields: name, base URL, the API key or Sign in with ChatGPT, the default
// model from the live list or typed, the context window, and for an OpenAI-compatible account its
// extra headers (accounts A3). Each row has Edit, Test, Make default, Move up and down, and Delete,
// which first names the agents that use the account (accounts A6). A ChatGPT account shows its
// plan's usage windows as bars (accounts A13), as its last answer reported them; a Test reads them
// afresh. Keys are stored by the server and never answered
// back (ai D7). The compaction settings are their own card. The chat's system prompt and the
// switches that let it make some kinds of change without asking are the built-in Assistant
// agent's (agents G6), so a line links to its agent screen. The section is web-app only: the
// desktop and Android apps talk to their embedded server, which holds no AI credentials.

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

export function modelLabel(m: AiModel): string {
  return m.display_name && m.display_name !== m.id ? `${m.display_name} (${m.id})` : m.id;
}

/** How long until `at`, from `now`, as "2 h 10 min" (accounts A13). */
export function untilText(at: number, now: number): string {
  const minutes = Math.ceil((at - now) / 60_000);
  if (minutes <= 0) return "now";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return minutes % 60 ? `${hours} h ${minutes % 60} min` : `${hours} h`;
  const days = Math.floor(hours / 24);
  return hours % 24 ? `${days} d ${hours % 24} h` : `${days} d`;
}

/** One usage window's line: "N% used, resets in 2 h 10 min". */
export function usageText(w: AiUsage["windows"][number], now: number): string {
  const used = `${Math.round(w.used_percent)}% used`;
  return w.resets_at_ms === null ? used : `${used}, resets in ${untilText(w.resets_at_ms, now)}`;
}

/** When a usage reading was taken: the time today, else the date and time. */
export function readAtText(ms: number, now: number): string {
  const at = new Date(ms);
  const today = at.toDateString() === new Date(now).toDateString();
  return today
    ? at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : at.toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}

/** A subscription's usage windows as bars, and when it was read (accounts A13). */
export function UsageBars(props: { usage: AiUsage; name: string }) {
  const now = () => Date.now();
  return (
    <div class="ai-usage" role="group" aria-label={`${props.name} usage`}>
      <For each={props.usage.windows}>
        {(w) => (
          <div class="ai-usage-window">
            <span class="ai-usage-label">{w.label}</span>
            <span
              class="ai-usage-bar"
              role="meter"
              aria-label={`${w.label} used`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(w.used_percent)}
            >
              <span style={{ width: `${Math.min(100, Math.max(0, w.used_percent))}%` }} />
            </span>
            <span class="ai-usage-text">{usageText(w, now())}</span>
          </div>
        )}
      </For>
      <span class="muted ai-usage-read">
        {props.usage.plan ? `${props.usage.plan} plan, as of ` : "As of "}
        {readAtText(props.usage.read_at_ms, now())}
      </span>
    </div>
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

/** An account's state, in a word or three. */
export function accountState(a: AiAccountStatus): string {
  if (a.kind === "chatgpt") {
    if (!a.signed_in) return "Not signed in";
    return a.signed_in.expired ? "Sign-in expired" : `Signed in as ${a.signed_in.account ?? "an unknown account"}`;
  }
  if (a.key_set) return "Key set";
  return a.configured ? "No key" : "Not set up";
}

/** The ChatGPT sign-in of one account (ai D8), through the paste-the-address step. */
function ChatGptSignIn(props: { account: AiAccountStatus; onStatus: (s: AiStatus) => void }) {
  const [authUrl, setAuthUrl] = createSignal<string | null>(null);
  const [pasted, setPasted] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<"start" | "finish" | "disconnect" | null>(null);
  const busy = () => pending() !== null;
  const doing = (a: "start" | "finish" | "disconnect") => pending() === a;
  const id = () => props.account.id;

  async function act(a: "start" | "finish" | "disconnect", f: () => Promise<void>) {
    setError(null);
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
      const { auth_url } = await api.chatgptStart(id());
      setAuthUrl(auth_url);
      setPasted("");
    });
  }

  function finish(ev: SubmitEvent) {
    ev.preventDefault();
    void act("finish", async () => {
      props.onStatus(await api.chatgptFinish(id(), pasted().trim()));
      setAuthUrl(null);
      setPasted("");
    });
  }

  function disconnect() {
    if (!confirm(`Disconnect ${props.account.name}? TheVault signs out of the plan and forgets the sign-in.`)) return;
    void act("disconnect", async () => props.onStatus(await api.chatgptDisconnect(id())));
  }

  return (
    <div class="ai-signin">
      <Show
        when={props.account.signed_in && !props.account.signed_in.expired}
        fallback={
          <Show
            when={authUrl()}
            fallback={
              <button type="button" class="primary" disabled={busy()} aria-busy={doing("start") ? "true" : undefined} onClick={start}>
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
                <label for={`chatgpt-pasted-${id()}`}>Address from the browser</label>
                <input
                  id={`chatgpt-pasted-${id()}`}
                  class="chatgpt-pasted"
                  autocomplete="off"
                  placeholder={`${CHATGPT_REDIRECT}?code=…&state=…`}
                  value={pasted()}
                  onInput={(e) => setPasted(e.currentTarget.value)}
                />
                <div class="actions">
                  <button type="submit" class="primary" disabled={busy() || !pasted().trim()} aria-busy={doing("finish") ? "true" : undefined}>
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
          Using a ChatGPT plan as {props.account.signed_in?.account ?? "an unknown account"}
        </p>
        <div class="actions">
          <a href={CHATGPT_USAGE_URL} target="_blank" rel="noopener noreferrer">
            Manage usage
          </a>
          <button type="button" class="danger" disabled={busy()} aria-busy={doing("disconnect") ? "true" : undefined} onClick={disconnect}>
            <Busy on={doing("disconnect")} />
            Disconnect
          </button>
        </div>
      </Show>
      <Show when={props.account.signed_in?.expired && !authUrl()}>
        <p class="error">The sign-in expired: sign in again.</p>
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </div>
  );
}

/** The model field: the account's live list once it can be read, else a typed id with what was listed as suggestions. */
function ModelField(props: {
  id: string;
  account: AiAccountStatus | null;
  value: string;
  onChange: (model: string) => void;
}) {
  const [models, setModels] = createSignal<AiModel[] | null>(null);
  const [preselected, setPreselected] = createSignal<string | null>(null);
  const [listError, setListError] = createSignal<string | null>(null);
  const ready = createMemo(() => (props.account?.configured ? props.account.id : null));
  const url = createMemo(() => props.account?.base_url ?? "");
  createEffect(
    on([ready, url], async ([id]) => {
      setListError(null);
      if (!id) {
        setModels(null);
        return;
      }
      try {
        const list = await api.aiModels(id);
        if (ready() !== id) return;
        setModels(list.models);
        setPreselected(list.model);
      } catch (e) {
        setModels(null);
        setListError(errorText(e));
      }
    }),
  );
  const listed = () => {
    const m = models();
    return m !== null && m.length > 0 && listError() === null ? m : null;
  };
  const placeholder = () => (preselected() ? `Default (${preselected()})` : "Default model");
  return (
    <>
      <Show
        when={listed()}
        fallback={
          <>
            <input
              id={props.id}
              list={`${props.id}-ids`}
              autocomplete="off"
              spellcheck={false}
              placeholder={props.account?.configured === false || !props.account ? "Type a model id, or leave empty" : placeholder()}
              value={props.value}
              onInput={(e) => props.onChange(e.currentTarget.value)}
            />
            <datalist id={`${props.id}-ids`}>
              <For each={(models() ?? []).map((m) => m.id)}>{(m) => <option value={m} />}</For>
            </datalist>
          </>
        }
      >
        {(list) => (
          <select id={props.id} value={props.value} onChange={(e) => props.onChange(e.currentTarget.value)}>
            <option value="">{placeholder()}</option>
            <Show when={props.value && !list().some((m) => m.id === props.value)}>
              <option value={props.value}>{props.value}</option>
            </Show>
            <For each={list()}>{(m) => <option value={m.id}>{modelLabel(m)}</option>}</For>
          </select>
        )}
      </Show>
      <Show when={listError()}>
        <p class="muted ai-model-note">The model list could not be read ({listError()}): type a model id.</p>
      </Show>
      <Show when={models()?.length === 0 && listError() === null}>
        <p class="muted ai-model-note">This service lists no models: type the id of the one to use.</p>
      </Show>
    </>
  );
}

/** A header pair as the editor holds it. */
interface HeaderRow {
  key: number;
  name: string;
  value: string;
}
let headerKeys = 0;

/**
 * The form that adds an account or edits one: the kind is chosen first for a new account, and
 * fixed for one that exists. Save writes the record, then the key when one was typed. A new
 * account's form becomes the account's own once it is added, for the ChatGPT sign-in and the live
 * model list.
 */
export function AccountForm(props: {
  status: AiStatus;
  /** The account edited, or undefined to add one. */
  account?: AiAccountStatus;
  onStatus: (s: AiStatus) => void;
  /** The account's id once it is saved, so a new account's form follows it. */
  onSaved: (id: string) => void;
  onClose: () => void;
}) {
  const editing = () => props.account;
  const [kind, setKind] = createSignal<AiKind | "">(props.account?.kind ?? "");
  const info = () => props.status.kinds.find((k) => k.kind === kind());
  const [name, setName] = createSignal(props.account?.name ?? "");
  /** Whether the name was typed, rather than following the kind. */
  let named = !!props.account;
  const [baseUrl, setBaseUrl] = createSignal(props.account?.base_url ?? "");
  const [key, setKey] = createSignal("");
  const [model, setModel] = createSignal(props.account?.model ?? "");
  const [windowText, setWindowText] = createSignal(props.account?.context?.window?.toString() ?? "");
  const [headers, setHeaders] = createSignal<HeaderRow[]>(
    (props.account?.headers ?? []).map(([n, v]) => ({ key: ++headerKeys, name: n, value: v })),
  );
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<"save" | "remove" | null>(null);
  const uid = () => props.account?.id ?? "new";
  const id = (what: string) => `ai-account-${uid()}-${what}`;
  const isChatGpt = () => kind() === "chatgpt";
  const compatible = () => kind() === "openai_compatible";

  function chooseKind(k: AiKind | "") {
    setKind(k);
    const label = props.status.kinds.find((x) => x.kind === k)?.label ?? "";
    // A name not typed yet follows the kind, made unique.
    if (!named || !name().trim()) setName(label ? uniqueName(label, props.status.accounts) : "");
  }

  function record() {
    const n = windowText().trim() === "" ? null : Number(windowText());
    return {
      name: name().trim(),
      base_url: isChatGpt() ? null : baseUrl().trim() || null,
      model: model().trim() || null,
      headers: compatible()
        ? headers()
            .filter((h) => h.name.trim())
            .map((h): [string, string] => [h.name.trim(), h.value])
        : [],
      context: n !== null && Number.isFinite(n) ? { window: n } : null,
    };
  }

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
    const k = kind();
    if (!k) {
      setError("Choose a kind first.");
      return;
    }
    const r = record();
    if (!r.name) {
      setError("Name the account.");
      return;
    }
    if (compatible() && !r.base_url) {
      setError("An OpenAI-compatible account needs its base URL.");
      return;
    }
    if (windowText().trim() !== "" && !r.context) {
      setError("The context window is a number of tokens.");
      return;
    }
    const typedKey = key().trim();
    void act("save", async () => {
      let accountId = editing()?.id;
      if (accountId) {
        props.onStatus(await api.aiUpdateAccount(accountId, r));
        if (typedKey && !isChatGpt()) {
          props.onStatus(await api.aiPutKey(accountId, typedKey));
          setKey("");
        }
      } else {
        // A new account takes its key with it.
        const made = await api.aiCreateAccount({ kind: k, ...r, ...(typedKey && !isChatGpt() ? { key: typedKey } : {}) });
        accountId = made.id;
        setKey("");
        props.onStatus(await api.aiStatus());
      }
      props.onSaved(accountId);
      return `Saved ${r.name}.`;
    });
  }

  function removeKey() {
    const a = editing();
    if (!a || !confirm(`Remove the API key of ${a.name}? TheVault forgets it.`)) return;
    void act("remove", async () => {
      props.onStatus(await api.aiPutKey(a.id, ""));
      return "Key removed.";
    });
  }

  const title = () => (editing() ? `Edit ${editing()!.name}` : "Add an account");

  return (
    <section class="field-card ai-card ai-account-form" aria-label={title()}>
      <header>
        <h2>{title()}</h2>
      </header>
      <form class="admin-form" aria-label={editing() ? `${editing()!.name} settings` : "New account"} onSubmit={save}>
        <div class="form-grid">
          <label for={id("kind")}>Kind</label>
          <Show when={!editing()} fallback={<span id={id("kind")}>{editing()!.kind_label}</span>}>
            <select id={id("kind")} value={kind()} onChange={(e) => chooseKind(e.currentTarget.value as AiKind | "")}>
              <option value="">Choose a kind</option>
              <For each={props.status.kinds}>{(k) => <option value={k.kind}>{k.label}</option>}</For>
            </select>
          </Show>
          <Show when={kind()}>
            <label for={id("name")}>Name</label>
            <input id={id("name")} maxLength={60} autocomplete="off" value={name()} onInput={(e) => {
                named = true;
                setName(e.currentTarget.value);
              }} />
            <Show when={!isChatGpt()}>
              <label for={id("url")}>Base URL</label>
              <input
                id={id("url")}
                type="url"
                autocomplete="off"
                spellcheck={false}
                placeholder={(editing() ? editing()!.default_base_url : info()?.default_url) ?? "https://openrouter.ai/api/v1"}
                value={baseUrl()}
                onInput={(e) => setBaseUrl(e.currentTarget.value)}
              />
              <label for={id("key")}>API key</label>
              <input
                id={id("key")}
                type="password"
                autocomplete="off"
                placeholder={editing()?.key_set ? "set" : compatible() ? "Optional: some services need none" : "Paste a key"}
                value={key()}
                onInput={(e) => setKey(e.currentTarget.value)}
              />
            </Show>
            <label for={id("model")}>Model</label>
            <div>
              <ModelField id={id("model")} account={editing() ?? null} value={model()} onChange={setModel} />
            </div>
            <label for={id("window")}>Context window, tokens</label>
            <input
              id={id("window")}
              type="number"
              min="4000"
              max="10000000"
              step="1000"
              placeholder={`${info()?.default_window ?? editing()?.default_window ?? ""}`}
              value={windowText()}
              onInput={(e) => setWindowText(e.currentTarget.value)}
            />
          </Show>
        </div>
        <Show when={kind() && !isChatGpt()}>
          <p class="muted">
            {compatible()
              ? "A service that speaks the OpenAI Chat Completions API, such as OpenRouter (https://openrouter.ai/api/v1) or Z.ai (https://api.z.ai/api/paas/v4)."
              : "Leave the URL empty for the official API, or give another service that speaks the same API."}
          </p>
        </Show>
        <Show when={compatible()}>
          <fieldset class="ai-headers">
            <legend>Extra headers</legend>
            <For each={headers()} fallback={<p class="muted">No extra headers.</p>}>
              {(h, i) => (
                <div class="ai-header-row">
                  <input
                    aria-label={`Header ${i() + 1} name`}
                    placeholder="HTTP-Referer"
                    autocomplete="off"
                    spellcheck={false}
                    value={h.name}
                    onInput={(e) => {
                      const v = e.currentTarget.value;
                      setHeaders((list) => list.map((x) => (x.key === h.key ? { ...x, name: v } : x)));
                    }}
                  />
                  <input
                    aria-label={`Header ${i() + 1} value`}
                    autocomplete="off"
                    spellcheck={false}
                    value={h.value}
                    onInput={(e) => {
                      const v = e.currentTarget.value;
                      setHeaders((list) => list.map((x) => (x.key === h.key ? { ...x, value: v } : x)));
                    }}
                  />
                  <button
                    type="button"
                    class="link"
                    aria-label={`Remove header ${i() + 1}`}
                    onClick={() => setHeaders((list) => list.filter((x) => x.key !== h.key))}
                  >
                    Remove
                  </button>
                </div>
              )}
            </For>
            <button type="button" onClick={() => setHeaders((list) => [...list, { key: ++headerKeys, name: "", value: "" }])}>
              Add header
            </button>
          </fieldset>
        </Show>
        <div class="actions">
          <button type="submit" class="primary" disabled={pending() !== null || !kind()} aria-busy={pending() === "save" ? "true" : undefined}>
            <Busy on={pending() === "save"} />
            {editing() ? "Save" : "Add account"}
          </button>
          <Show when={editing()?.key_set}>
            <button type="button" class="danger" disabled={pending() !== null} aria-busy={pending() === "remove" ? "true" : undefined} onClick={removeKey}>
              <Busy on={pending() === "remove"} />
              Remove key
            </button>
          </Show>
          <button type="button" onClick={() => props.onClose()}>
            {editing() ? "Close" : "Cancel"}
          </button>
        </div>
      </form>
      <Show when={isChatGpt()}>
        <Show when={editing()} fallback={<p class="muted">Add the account, then sign in with ChatGPT.</p>}>
          {(a) => <ChatGptSignIn account={a()} onStatus={props.onStatus} />}
        </Show>
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={notice()}>
        <p class="muted" role="status">
          {notice()}
        </p>
      </Show>
    </section>
  );
}

/** `base`, or `base 2`, `base 3`… when an account has that name already (names are unique, accounts A2). */
export function uniqueName(base: string, accounts: { name: string }[]): string {
  const taken = new Set(accounts.map((a) => a.name.toLowerCase()));
  if (!taken.has(base.toLowerCase())) return base;
  for (let n = 2; ; n++) if (!taken.has(`${base} ${n}`.toLowerCase())) return `${base} ${n}`;
}

/** The accounts table (accounts A11). */
export function AccountsCard(props: { status: AiStatus; onStatus: (s: AiStatus) => void }) {
  /** The account whose form is open, "new" for the Add form, or null. */
  const [open, setOpen] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [results, setResults] = createSignal<Record<string, AiTestResult>>({});
  const [pending, setPending] = createSignal<string | null>(null);
  const accounts = () => props.status.accounts;
  const editing = () => accounts().find((a) => a.id === open());

  async function act(what: string, f: () => Promise<void>) {
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

  function test(a: AiAccountStatus) {
    setResults(({ [a.id]: _, ...rest }) => rest);
    void act(`test:${a.id}`, async () => {
      const r = await api.aiTest(a.id);
      setResults((all) => ({ ...all, [a.id]: r }));
      // A Test with no model chosen saves the preselected one, and a plan's answer its usage.
      if (!a.model || a.kind === "chatgpt") props.onStatus(await api.aiStatus());
    });
  }

  function move(a: AiAccountStatus, by: number) {
    const ids = accounts().map((x) => x.id);
    const at = ids.indexOf(a.id);
    const to = at + by;
    if (to < 0 || to >= ids.length) return;
    [ids[at], ids[to]] = [ids[to]!, ids[at]!];
    void act(`move:${a.id}`, async () => props.onStatus(await api.aiOrderAccounts(ids)));
  }

  function remove(a: AiAccountStatus) {
    const users = a.agents;
    const ask = users.length
      ? `Delete ${a.name}? ${users.length === 1 ? "The agent" : "The agents"} ${users.join(", ")} ${users.length === 1 ? "names" : "name"} it and will use the default account instead.`
      : `Delete ${a.name}? TheVault forgets its key or sign-in.`;
    if (!confirm(ask)) return;
    void act(`delete:${a.id}`, async () => {
      props.onStatus(await api.aiDeleteAccount(a.id));
      if (open() === a.id) setOpen(null);
    });
  }

  return (
    <Card title="Accounts" blurb="The AI accounts scripts, agents and the chat run on. Agents name an account; those that name none use the default.">
      <Show when={accounts().length > 0} fallback={<p class="muted">No AI account yet. Add one to use the chat and agents.</p>}>
        <table class="admin-table ai-accounts" aria-label="AI accounts">
          <thead>
            <tr>
              <th>Name</th>
              <th>Kind</th>
              <th>Model</th>
              <th>Status</th>
              <th>Default</th>
              <th />
            </tr>
          </thead>
          <tbody>
            <For each={accounts()}>
              {(a, i) => (
                <>
                  <tr data-account={a.id} aria-label={a.name}>
                    <td>{a.name}</td>
                    <td>{a.kind_label}</td>
                    <td>{a.model ?? <span class="muted">Kind default</span>}</td>
                    <td>{accountState(a)}</td>
                    <td>
                      <Show
                        when={props.status.default === a.id}
                        fallback={
                          <button
                            type="button"
                            class="link"
                            aria-label={`Make ${a.name} the default`}
                            disabled={pending() !== null}
                            onClick={() => void act(`default:${a.id}`, async () => props.onStatus(await api.aiSetDefault(a.id)))}
                          >
                            Make default
                          </button>
                        }
                      >
                        <span class="ai-default-mark">Default</span>
                      </Show>
                    </td>
                    <td class="actions">
                      <button type="button" aria-label={`Edit ${a.name}`} onClick={() => setOpen(open() === a.id ? null : a.id)}>
                        Edit
                      </button>
                      <button
                        type="button"
                        aria-label={`Test ${a.name}`}
                        disabled={pending() !== null || !a.configured}
                        aria-busy={pending() === `test:${a.id}` ? "true" : undefined}
                        onClick={() => test(a)}
                      >
                        <Busy on={pending() === `test:${a.id}`} />
                        Test
                      </button>
                      <button type="button" class="link" aria-label={`Move ${a.name} up`} disabled={i() === 0 || pending() !== null} onClick={() => move(a, -1)}>
                        Up
                      </button>
                      <button
                        type="button"
                        class="link"
                        aria-label={`Move ${a.name} down`}
                        disabled={i() === accounts().length - 1 || pending() !== null}
                        onClick={() => move(a, 1)}
                      >
                        Down
                      </button>
                      <button type="button" class="danger" aria-label={`Delete ${a.name}`} disabled={pending() !== null} onClick={() => remove(a)}>
                        Delete
                      </button>
                    </td>
                  </tr>
                  <Show when={results()[a.id] || (a.kind === "chatgpt" && a.usage)}>
                    <tr class="ai-account-extra">
                      <td colSpan={6}>
                        <Show when={results()[a.id]}>
                          {(r) => (
                            <p class={r().ok ? "ai-test-ok" : "error"} role="status">
                              {a.name}: {testText(r())}
                            </p>
                          )}
                        </Show>
                        <Show when={a.kind === "chatgpt" && a.usage}>{(u) => <UsageBars usage={u()} name={a.name} />}</Show>
                      </td>
                    </tr>
                  </Show>
                </>
              )}
            </For>
          </tbody>
        </table>
      </Show>
      <Show when={open() === null}>
        <button type="button" class="primary" onClick={() => setOpen("new")}>
          Add account
        </button>
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={open() === "new"}>
        <AccountForm status={props.status} onStatus={props.onStatus} onSaved={(id) => setOpen(id)} onClose={() => setOpen(null)} />
      </Show>
      <Show when={editing()?.id} keyed>
        {(_) => <AccountForm status={props.status} account={editing()} onStatus={props.onStatus} onSaved={() => undefined} onClose={() => setOpen(null)} />}
      </Show>
    </Card>
  );
}

/** When a long chat is compacted; each account has its own window. */
export function ContextCard(props: { status: AiStatus; onStatus: (s: AiStatus) => void }) {
  const c = () => props.status.context;
  const [error, setError] = createSignal<string | null>(null);
  const [saved, setSaved] = createSignal<string | null>(null);
  async function save(body: Partial<AiContext>) {
    setError(null);
    setSaved(null);
    try {
      props.onStatus(await api.aiSaveSettings({ context: body }));
      setSaved("Saved.");
    } catch (e) {
      setError(errorText(e));
    }
  }
  return (
    <Card
      title="Context"
      blurb={
        <>
          When a long chat is summarized for the model. Each account's window is on its form. Type <code>/compact</code> in a
          chat to compact it at once.
        </>
      }
    >
      <div class="ai-context">
        <label class="ai-context-switch">
          <input type="checkbox" checked={c().auto_compact} onChange={(e) => void save({ auto_compact: e.currentTarget.checked })} />
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
            onChange={(e) => void save({ compact_at: e.currentTarget.valueAsNumber })}
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
        Scripts, agents and the AI panel call these accounts with your own credentials. The server keeps them encrypted
        and never shows them again.
      </p>
      <p class="ai-agent-link">
        The chat panel's prompt and permissions are on the <a href={`/agents/${ASSISTANT}`}>Assistant agent</a>.
      </p>
      <Show when={status()}>
        {(s) => (
          <>
            <Show when={!s().secrets_key}>
              <p class="error" role="alert">
                This server has no secrets key, so it cannot store AI credentials.
              </p>
            </Show>
            <AccountsCard status={s()} onStatus={setStatus} />
            <ContextCard status={s()} onStatus={setStatus} />
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

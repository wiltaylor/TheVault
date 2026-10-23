import { A, useLocation, useNavigate } from "@solidjs/router";
import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import {
  type BraveStatus,
  type GoogleRedirect,
  type GoogleSettings,
  type GoogleSide,
  type GoogleStatus,
  api,
  waitForRun,
} from "../api";
import { ChipPicker } from "../components/ChipPicker";
import { Spinner } from "../components/SyncStatus";
import { errorText } from "./TypeEditor";

// Settings > Integrations (google-sync D1, D18, D21): the Google card signs in with the user's own
// OAuth client, shows how each side of the sync is doing, and edits what it pulls. The settings
// are a secret on the server, not a note, so this card is their only editor. On an https page Google sends the
// browser straight back to /integrations/google/callback; on http, where Google accepts no
// redirect but a loopback one, the user pastes the address the browser failed to load.
// Every button shows a spinner while its request is in flight, and a side that is running shows
// "Syncing…" while the card polls the status every few seconds (ticket 113). Run now answers as
// soon as the run starts; the card then reads the run until it ends and says how it went, while
// its other buttons stay free (ticket 115).
//
// The Brave Search card (ai D26) stores the API key the AI chat's web_search tool uses: the key is
// never shown back, only whether one is set; Test searches once for "TheVault".

/** How often the card reads the status again while a side is running. */
export const POLL_MS = 3000;

/** Where the Google Cloud setup is written down. */
export const GOOGLE_SETUP_DOC = "https://github.com/wiltaylor/TheVault/blob/main/docs/google.md";

/** How a page at `protocol` signs in: directly on https, through a pasted loopback address otherwise. */
export function redirectMode(protocol: string): GoogleRedirect {
  return protocol === "https:" ? "direct" : "loopback";
}

const SIDES = [
  ["gmail", "Gmail"],
  ["calendar", "Calendar"],
] as const;

function whenText(iso: string | null): string {
  if (!iso) return "Never";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

function stateText(s: GoogleSide): string {
  if (!s.installed) return "Not installed";
  if (s.paused === null) return "Broken task file";
  return s.paused ? "Paused" : "Scheduled";
}

/** What the Google sync pulls: Gmail mode, labels, window and back-fill date, and the calendars. */
export function GoogleSettingsForm(props: {
  settings: GoogleSettings;
  busy: boolean;
  /** Whether the save is in flight, which the Save button shows. */
  saving?: boolean;
  onSave: (settings: GoogleSettings) => void;
}) {
  const [mode, setMode] = createSignal(props.settings.gmail_mode);
  const [labels, setLabels] = createSignal(props.settings.gmail_labels);
  const [days, setDays] = createSignal(String(props.settings.gmail_window_days));
  const [backfill, setBackfill] = createSignal(props.settings.gmail_backfill_to ?? "");
  const [calendars, setCalendars] = createSignal(props.settings.calendars);
  const [defaultCalendar, setDefaultCalendar] = createSignal(props.settings.default_calendar);
  const [problem, setProblem] = createSignal<string | null>(null);

  function save(ev: SubmitEvent) {
    ev.preventDefault();
    const n = Number(days().trim());
    if (!Number.isInteger(n) || n < 1 || n > 3650) {
      setProblem("The Gmail window is a whole number of days, 1 to 3650.");
      return;
    }
    if (calendars().length === 0) {
      setProblem("List at least one calendar, such as primary.");
      return;
    }
    setProblem(null);
    props.onSave({
      gmail_mode: mode(),
      gmail_labels: labels(),
      gmail_window_days: n,
      gmail_backfill_to: backfill() || null,
      calendars: calendars(),
      default_calendar: defaultCalendar().trim() || (calendars()[0] ?? "primary"),
    });
  }

  return (
    <form class="admin-form google-settings" aria-label="Google sync settings" novalidate onSubmit={save}>
      <h3>What to sync</h3>
      <div class="form-grid">
        <label for="google-gmail-mode">Gmail mode</label>
        <select
          id="google-gmail-mode"
          value={mode()}
          onChange={(e) => setMode(e.currentTarget.value as GoogleSettings["gmail_mode"])}
        >
          <option value="labels">The labels below</option>
          <option value="all">All mail</option>
        </select>
        <span id="google-labels-label">Gmail labels</span>
        <ChipPicker
          label="Gmail labels"
          labelledBy="google-labels-label"
          values={labels()}
          freeText
          suggest={() => []}
          disabled={mode() === "all"}
          placeholder="A label, such as INBOX"
          emptyText="No labels yet"
          onChange={setLabels}
        />
        <span />
        <p class="muted">Changing the labels or the mode lists the window again on the next run.</p>
        <label for="google-window-days">Window (days)</label>
        <input
          id="google-window-days"
          type="number"
          min="1"
          max="3650"
          value={days()}
          onInput={(e) => setDays(e.currentTarget.value)}
        />
        <label for="google-backfill-to">Back-fill to</label>
        <input
          id="google-backfill-to"
          type="date"
          value={backfill()}
          onInput={(e) => setBackfill(e.currentTarget.value)}
        />
        <span id="google-calendars-label">Calendars</span>
        <ChipPicker
          label="Calendars"
          labelledBy="google-calendars-label"
          values={calendars()}
          freeText
          suggest={() => []}
          placeholder="A calendar id, such as primary"
          onChange={setCalendars}
        />
        <label for="google-default-calendar">Default calendar</label>
        <input
          id="google-default-calendar"
          autocomplete="off"
          value={defaultCalendar()}
          onInput={(e) => setDefaultCalendar(e.currentTarget.value)}
        />
      </div>
      <Show when={problem()}>
        <p class="error" role="alert">
          {problem()}
        </p>
      </Show>
      <button type="submit" class="primary" disabled={props.busy} aria-busy={props.saving ? "true" : undefined}>
        <Busy on={!!props.saving} />
        Save
      </button>
    </form>
  );
}

/** The spinner a button shows while its request is in flight. */
function Busy(props: { on: boolean }) {
  return (
    <Show when={props.on}>
      <Spinner />{" "}
    </Show>
  );
}

/** The request in flight: which button started it. */
type Action = "connect" | "finish" | "save" | "disconnect" | "run:gmail" | "run:calendar";

/**
 * The Google card. `protocol` and `assign` are the page's own unless a test gives others, and
 * `pollMs` is [`POLL_MS`] unless a test gives another.
 */
export function GoogleCard(props: { protocol?: string; assign?: (url: string) => void; pollMs?: number }) {
  const mode = () => redirectMode(props.protocol ?? window.location.protocol);
  const assign = (url: string) => (props.assign ?? ((u: string) => window.location.assign(u)))(url);
  const [status, setStatus] = createSignal<GoogleStatus | null>(null);
  const [clientId, setClientId] = createSignal("");
  const [clientSecret, setClientSecret] = createSignal("");
  const [authUrl, setAuthUrl] = createSignal<string | null>(null);
  const [pasted, setPasted] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<Action | null>(null);
  const busy = () => pending() !== null;
  const doing = (a: Action) => pending() === a;
  /** The sides whose Run now the card is waiting on. */
  const [watching, setWatching] = createSignal<Partial<Record<(typeof SIDES)[number][0], boolean>>>({});
  /** Whether `key`'s side is running: the server says so, or its Run now is in flight or going. */
  const running = (key: (typeof SIDES)[number][0]) =>
    !!status()?.[key].running || doing(`run:${key}`) || !!watching()[key];
  let disposed = false;
  onCleanup(() => (disposed = true));

  async function load() {
    try {
      setStatus(await api.googleStatus());
    } catch (e) {
      setError(errorText(e));
    }
  }
  onMount(load);

  // While either side runs, read the status again every few seconds, until neither does.
  createEffect(() => {
    if (!status() || !SIDES.some(([key]) => running(key))) return;
    const timer = setTimeout(() => void load(), props.pollMs ?? POLL_MS);
    onCleanup(() => clearTimeout(timer));
  });

  async function act(action: Action, f: () => Promise<void>) {
    setError(null);
    setNotice(null);
    setPending(action);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPending(null);
    }
  }

  function connect(ev: SubmitEvent) {
    ev.preventDefault();
    const id = clientId().trim();
    const secret = clientSecret().trim();
    if (!id || !secret) {
      setError("Enter the OAuth client's client ID and client secret from Google Cloud.");
      return;
    }
    void act("connect", async () => {
      const { auth_url } = await api.googleStart(id, secret, mode());
      if (mode() === "direct") {
        assign(auth_url);
        return;
      }
      setAuthUrl(auth_url);
      setPasted("");
    });
  }

  function finish(ev: SubmitEvent) {
    ev.preventDefault();
    void act("finish", async () => {
      setStatus(await api.googleFinish(pasted().trim()));
      setAuthUrl(null);
      setPasted("");
      setClientSecret("");
    });
  }

  function disconnect() {
    if (!confirm("Disconnect Google? The sync stops and TheVault forgets the sign-in. Notes already synced stay.")) return;
    void act("disconnect", async () => {
      setStatus(await api.googleDisconnect());
      setNotice("Disconnected. The Gmail and Calendar sync are paused.");
    });
  }

  function saveSettings(settings: GoogleSettings) {
    void act("save", async () => {
      setStatus(await api.googleSaveSettings(settings));
      setNotice("Saved. The next runs use these settings.");
    });
  }

  function runNow(key: (typeof SIDES)[number][0], label: string, task: string) {
    void act(`run:${key}`, async () => {
      const started = await api.runTask(task);
      setWatching((w) => ({ ...w, [key]: true }));
      void (async () => {
        try {
          const run = await waitForRun(started, (id) => api.taskRun(task, id), props.pollMs ?? POLL_MS, () => disposed);
          if (disposed || run.outcome === "running") return;
          setNotice(run.outcome === "failed" ? `${label} ran and failed: ${run.message ?? "see its run log"}` : `${label} ran.`);
          await load();
        } catch (e) {
          setError(errorText(e));
        } finally {
          setWatching((w) => ({ ...w, [key]: false }));
        }
      })();
      await load();
    });
  }

  return (
    <section class="field-card google-card" aria-label="Google">
      <header>
        <h2>Google</h2>
        <p class="muted">Gmail and Google Calendar, signed in with your own Google Cloud OAuth client.</p>
      </header>
      <Show when={status()}>
        {(s) => (
          <>
            <Show
              when={s().connected}
              fallback={
                <Show
                  when={authUrl()}
                  fallback={
                    <form class="admin-form" aria-label="Connect Google" onSubmit={connect}>
                      <p>
                        <a href={GOOGLE_SETUP_DOC} target="_blank" rel="noopener noreferrer">
                          How to set up a Google OAuth client
                        </a>{" "}
                        ({mode() === "direct" ? "a Web application client with this page's callback" : "a Desktop app client"}).
                      </p>
                      <div class="form-grid">
                        <label for="google-client-id">Client ID</label>
                        <input
                          id="google-client-id"
                          autocomplete="off"
                          placeholder="1234-abc.apps.googleusercontent.com"
                          value={clientId()}
                          onInput={(e) => setClientId(e.currentTarget.value)}
                        />
                        <label for="google-client-secret">Client secret</label>
                        <input
                          id="google-client-secret"
                          type="password"
                          autocomplete="new-password"
                          value={clientSecret()}
                          onInput={(e) => setClientSecret(e.currentTarget.value)}
                        />
                      </div>
                      <Show when={s().client_stored}>
                        <p class="muted">A client is stored already; enter it again to connect.</p>
                      </Show>
                      <button
                        type="submit"
                        class="primary"
                        disabled={busy()}
                        aria-busy={doing("connect") ? "true" : undefined}
                      >
                        <Busy on={doing("connect")} />
                        Connect Google
                      </button>
                    </form>
                  }
                >
                  {(url) => (
                    <form class="admin-form" aria-label="Finish connecting Google" onSubmit={finish}>
                      <ol class="google-steps">
                        <li>
                          <a href={url()} target="_blank" rel="noopener noreferrer">
                            Open Google's sign-in page
                          </a>{" "}
                          and approve. Google warns the app is unverified: choose Advanced, then Go to the app.
                        </li>
                        <li>
                          The browser then fails to load a <code>127.0.0.1</code> page. That is expected: copy the whole
                          address from its address bar.
                        </li>
                        <li>Paste it here within a few minutes.</li>
                      </ol>
                      <label for="google-pasted">Address from the browser</label>
                      <input
                        id="google-pasted"
                        autocomplete="off"
                        placeholder="http://127.0.0.1:53682/?state=…&code=…"
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
                          Finish connecting
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
                Connected as {s().account ?? "an unknown address"}
              </p>
            </Show>
            <table class="admin-table" aria-label="Google sync">
              <thead>
                <tr>
                  <th>Sync</th>
                  <th>Schedule</th>
                  <th>Last run</th>
                  <th>Last error</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                <For each={SIDES}>
                  {([key, label]) => {
                    const side = () => s()[key];
                    return (
                      <tr aria-label={label}>
                        <td>{label}</td>
                        <td>{stateText(side())}</td>
                        <td>
                          <Show
                            when={running(key)}
                            fallback={
                              <>
                                {whenText(side().last_run)}
                                <Show when={side().summary}>
                                  <div class="muted">{side().summary}</div>
                                </Show>
                              </>
                            }
                          >
                            <span class="google-syncing" role="status" aria-busy="true">
                              <Spinner /> Syncing…
                            </span>
                          </Show>
                        </td>
                        <td classList={{ error: !!side().last_error }}>{side().last_error ?? "No errors"}</td>
                        <td class="actions">
                          <Show when={s().connected && side().installed}>
                            <button
                              type="button"
                              disabled={busy() || running(key)}
                              aria-busy={doing(`run:${key}`) ? "true" : undefined}
                              onClick={() => runNow(key, label, side().task)}
                            >
                              <Busy on={doing(`run:${key}`)} />
                              Run {label} now
                            </button>
                          </Show>
                        </td>
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
            <GoogleSettingsForm settings={s().settings} busy={busy()} saving={doing("save")} onSave={saveSettings} />
            <Show when={s().connected}>
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
          </>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={notice()}>
        <p class="muted">{notice()}</p>
      </Show>
    </section>
  );
}

/** Where a Brave Search API key comes from. */
export const BRAVE_DASHBOARD = "https://api-dashboard.search.brave.com/";

/** The request in flight on the Brave card. */
type BraveAction = "save" | "remove" | "test";

/** The Brave Search card: the key the AI chat's web_search tool uses (ai D26). */
export function BraveCard() {
  const [status, setStatus] = createSignal<BraveStatus | null>(null);
  const [key, setKey] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<BraveAction | null>(null);
  const busy = () => pending() !== null;
  const doing = (a: BraveAction) => pending() === a;
  const set = () => !!status()?.configured;

  onMount(async () => {
    try {
      setStatus(await api.braveStatus());
    } catch (e) {
      setError(errorText(e));
    }
  });

  async function act(action: BraveAction, f: () => Promise<string | null>) {
    setError(null);
    setNotice(null);
    setPending(action);
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
    if (!k) {
      setError("Paste the Brave Search API key first.");
      return;
    }
    void act("save", async () => {
      setStatus(await api.bravePutKey(k));
      setKey("");
      return "Saved. The AI chat can search the web.";
    });
  }

  function remove() {
    if (!confirm("Remove the Brave Search API key? TheVault forgets it, and the AI chat stops searching the web.")) return;
    void act("remove", async () => {
      setStatus(await api.bravePutKey(""));
      setKey("");
      return "Removed. The AI chat no longer searches the web.";
    });
  }

  function test() {
    void act("test", async () => {
      const r = await api.braveTest();
      if (!r.ok) throw new Error(r.error?.message ?? "The test search failed.");
      return `Test search worked: ${r.count ?? 0} result${r.count === 1 ? "" : "s"} for “TheVault” in ${r.ms} ms.`;
    });
  }

  return (
    <section class="field-card brave-card" aria-label="Brave Search">
      <header>
        <h2>Brave Search</h2>
        <p class="muted">Lets the AI chat search the web, with your own Brave Search API key.</p>
      </header>
      <Show when={status()}>
        <form class="admin-form" aria-label="Brave Search key" onSubmit={save}>
          <p>
            <a href={BRAVE_DASHBOARD} target="_blank" rel="noopener noreferrer">
              Get a Brave Search API key
            </a>
          </p>
          <div class="form-grid">
            <label for="brave-key">API key</label>
            <input
              id="brave-key"
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
              disabled={busy() || !key().trim()}
              aria-busy={doing("save") ? "true" : undefined}
            >
              <Busy on={doing("save")} />
              Save
            </button>
            <Show when={set()}>
              <button type="button" disabled={busy()} aria-busy={doing("test") ? "true" : undefined} onClick={test}>
                <Busy on={doing("test")} />
                Test
              </button>
              <button
                type="button"
                class="danger"
                disabled={busy()}
                aria-busy={doing("remove") ? "true" : undefined}
                onClick={remove}
              >
                <Busy on={doing("remove")} />
                Remove key
              </button>
            </Show>
          </div>
        </form>
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

/** Settings > Integrations. */
export function Integrations() {
  return (
    <section class="list-page">
      <header class="note-header">
        <h1>Integrations</h1>
      </header>
      <GoogleCard />
      <BraveCard />
    </section>
  );
}

/**
 * Where Google sends the browser back on an https page: posts its own address to finish the
 * sign-in, then returns to the Integrations page in place of itself.
 */
export function GoogleCallback() {
  const location = useLocation();
  const navigate = useNavigate();
  const [error, setError] = createSignal<string | null>(null);
  onMount(async () => {
    try {
      await api.googleFinish(`${window.location.origin}${location.pathname}${location.search}`);
      navigate("/settings/integrations", { replace: true });
    } catch (e) {
      setError(errorText(e));
    }
  });
  return (
    <section class="list-page">
      <h1>Connecting Google</h1>
      <Show when={error()} fallback={<p class="muted">Finishing the sign-in…</p>}>
        <p class="error" role="alert">
          {error()}
        </p>
        <p>
          <A href="/settings/integrations">Back to Integrations</A>
        </p>
      </Show>
    </section>
  );
}

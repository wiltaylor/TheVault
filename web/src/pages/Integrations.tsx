import { A, useLocation, useNavigate } from "@solidjs/router";
import { For, Show, createSignal, onMount } from "solid-js";
import { type GoogleRedirect, type GoogleSettings, type GoogleSide, type GoogleStatus, api } from "../api";
import { ChipPicker } from "../components/ChipPicker";
import { errorText } from "./TypeEditor";

// Settings > Integrations (google-sync D1, D18, D21): the Google card signs in with the user's own
// OAuth client, shows how each side of the sync is doing, and edits what it pulls. The settings
// are a secret on the server, not a note, so this card is their only editor. On an https page Google sends the
// browser straight back to /integrations/google/callback; on http, where Google accepts no
// redirect but a loopback one, the user pastes the address the browser failed to load.

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
  return s.paused ? "Paused" : "Syncing";
}

/** What the Google sync pulls: Gmail mode, labels, window and back-fill date, and the calendars. */
export function GoogleSettingsForm(props: {
  settings: GoogleSettings;
  busy: boolean;
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
      <button type="submit" class="primary" disabled={props.busy}>
        Save
      </button>
    </form>
  );
}

/** The Google card. `protocol` and `assign` are the page's own unless a test gives others. */
export function GoogleCard(props: { protocol?: string; assign?: (url: string) => void }) {
  const mode = () => redirectMode(props.protocol ?? window.location.protocol);
  const assign = (url: string) => (props.assign ?? ((u: string) => window.location.assign(u)))(url);
  const [status, setStatus] = createSignal<GoogleStatus | null>(null);
  const [clientId, setClientId] = createSignal("");
  const [clientSecret, setClientSecret] = createSignal("");
  const [authUrl, setAuthUrl] = createSignal<string | null>(null);
  const [pasted, setPasted] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  async function load() {
    try {
      setStatus(await api.googleStatus());
    } catch (e) {
      setError(errorText(e));
    }
  }
  onMount(load);

  async function act(f: () => Promise<void>) {
    setError(null);
    setNotice(null);
    setBusy(true);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
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
    void act(async () => {
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
    void act(async () => {
      setStatus(await api.googleFinish(pasted().trim()));
      setAuthUrl(null);
      setPasted("");
      setClientSecret("");
    });
  }

  function disconnect() {
    if (!confirm("Disconnect Google? The sync stops and TheVault forgets the sign-in. Notes already synced stay.")) return;
    void act(async () => {
      setStatus(await api.googleDisconnect());
      setNotice("Disconnected. The Gmail and Calendar sync are paused.");
    });
  }

  function saveSettings(settings: GoogleSettings) {
    void act(async () => {
      setStatus(await api.googleSaveSettings(settings));
      setNotice("Saved. The next runs use these settings.");
    });
  }

  function runNow(label: string, task: string) {
    void act(async () => {
      const run = await api.runTask(task);
      setNotice(run.outcome === "failed" ? `${label} ran and failed: ${run.message ?? "see its run log"}` : `${label} ran.`);
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
                      <button type="submit" class="primary" disabled={busy()}>
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
                        <button type="submit" class="primary" disabled={busy() || !pasted().trim()}>
                          Finish connecting
                        </button>
                        <button type="button" onClick={() => setAuthUrl(null)}>
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
                        <td>{whenText(side().last_run)}</td>
                        <td classList={{ error: !!side().last_error }}>{side().last_error ?? "No errors"}</td>
                        <td class="actions">
                          <Show when={s().connected && side().installed}>
                            <button type="button" disabled={busy()} onClick={() => runNow(label, side().task)}>
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
            <GoogleSettingsForm settings={s().settings} busy={busy()} onSave={saveSettings} />
            <Show when={s().connected}>
              <button type="button" class="danger" disabled={busy()} onClick={disconnect}>
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

/** Settings > Integrations. */
export function Integrations() {
  return (
    <section class="list-page">
      <header class="note-header">
        <h1>Integrations</h1>
      </header>
      <GoogleCard />
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

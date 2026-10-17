import { A, useLocation, useNavigate } from "@solidjs/router";
import { For, Show, createSignal, onMount } from "solid-js";
import { type GoogleRedirect, type GoogleSide, type GoogleStatus, api } from "../api";
import { errorText } from "./TypeEditor";

// Settings > Integrations (google-sync D1, D18): the Google card signs in with the user's own
// OAuth client and shows how each side of the sync is doing. On an https page Google sends the
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

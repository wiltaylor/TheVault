import { Show, createResource, createSignal, onCleanup } from "solid-js";
import { type AppInfo, ApiError, api } from "../api";
import { type UpdateState, checkForUpdates, openDownload, restartToUpdate, updateState } from "../appUpdates";
import { appInfo } from "../inApp";
import { webBuild } from "../version";

/** How long "Check for updates" waits for the check to finish before it stops watching. */
const CHECK_WAIT_MS = 60_000;
const CHECK_POLL_MS = 1_500;

const PLATFORMS: Record<string, string> = {
  "linux-appimage": "Linux AppImage",
  "linux-package": "Linux deb/rpm package",
  linux: "Linux",
  android: "Android",
};

/** The platform an app reports, as people say it. */
export function platformLabel(platform: string | null | undefined): string {
  if (!platform) return "Unknown";
  return PLATFORMS[platform] ?? platform;
}

/** The home server's version, or why it cannot be read. */
async function serverVersion(): Promise<string> {
  try {
    return await api.serverVersion();
  } catch (e) {
    if (e instanceof ApiError && e.body.error === "not_paired") return "Not paired";
    return "unreachable";
  }
}

function when(ms: number | null): string {
  return ms === null ? "never" : new Date(ms).toLocaleString();
}

/** Settings > General's About section: which app, server and web build this is. */
export function About() {
  const [app] = createResource(appInfo);
  const [server] = createResource(serverVersion);
  return (
    <div class="settings-form about" role="group" aria-label="About">
      <h2>About</h2>
      <dl class="about-list">
        <Show
          when={app()}
          fallback={
            <Show when={app.state === "ready"}>
              <dt>App</dt>
              <dd data-about="app">Web app</dd>
            </Show>
          }
        >
          {(info) => (
            <>
              <dt>App</dt>
              <dd data-about="app">TheVault {info().version ?? "(version unknown)"}</dd>
              <dt>Platform</dt>
              <dd data-about="platform">{platformLabel(info().platform)}</dd>
              <dt>Build</dt>
              <dd data-about="build">{info().build ?? "Unknown"}</dd>
              <dt>Device</dt>
              <dd data-about="device">{info().device_name ?? "Not paired"}</dd>
            </>
          )}
        </Show>
        <dt>Server</dt>
        <dd data-about="server">{server() ?? "…"}</dd>
        <dt>Web build</dt>
        <dd data-about="web">{webBuild()}</dd>
      </dl>
      <Show when={app()}>{(info) => <Updates info={info()} />}</Show>
    </div>
  );
}

/** The app update check: where it stands, and "Check for updates". */
function Updates(props: { info: AppInfo }) {
  const [state, { mutate }] = createResource(
    () => props.info,
    (info) => updateState(info).catch((): UpdateState | null => null),
  );
  const [waiting, setWaiting] = createSignal(false);
  const [failed, setFailed] = createSignal<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  async function check() {
    setFailed(null);
    const before = state()?.checked ?? null;
    setWaiting(true);
    try {
      await checkForUpdates(props.info);
    } catch (e) {
      setWaiting(false);
      setFailed(e instanceof Error ? e.message : String(e));
      return;
    }
    const until = Date.now() + CHECK_WAIT_MS;
    const poll = async () => {
      const now = await updateState(props.info).catch(() => null);
      if (now) mutate(now);
      const done = now !== null && !now.checking && now.checked !== before;
      if (done || Date.now() > until) {
        setWaiting(false);
        return;
      }
      timer = setTimeout(() => void poll(), CHECK_POLL_MS);
    };
    timer = setTimeout(() => void poll(), CHECK_POLL_MS);
  }

  async function run(action: () => Promise<void>) {
    setFailed(null);
    try {
      await action();
    } catch (e) {
      setFailed(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div class="about-updates" role="group" aria-label="Updates">
      <h3>Updates</h3>
      <Show when={state()} fallback={<p class="muted">{state.loading ? "…" : "The update status is not available."}</p>}>
        {(s) => (
          <>
            <p data-about="updates" role="status">
              {waiting() || s().checking ? "Checking for updates…" : s().summary}
            </p>
            <p class="muted">
              Last checked: {when(s().checked)}
              <Show when={s().automaticOff}>. Automatic checks are off.</Show>
            </p>
            <div class="settings-actions">
              <Show when={s().canCheck}>
                <button type="button" disabled={waiting() || s().checking} onClick={() => void check()}>
                  Check for updates
                </button>
              </Show>
              <Show when={s().restart}>
                <button type="button" onClick={() => void run(restartToUpdate)}>
                  Restart to update
                </button>
              </Show>
              <Show when={s().download}>
                <button type="button" onClick={() => void run(openDownload)}>
                  Download
                </button>
              </Show>
            </div>
          </>
        )}
      </Show>
      <Show when={failed()}>
        <p class="error">{failed()}</p>
      </Show>
    </div>
  );
}

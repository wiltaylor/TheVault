import { For, Index, Show, createResource, createSignal } from "solid-js";
import { type Device, type DeviceExclusion, type DeviceFileInfo, type FreshStart, type PairCode, api } from "../api";
import { when } from "../admin/format";
import { inAndroidApp } from "../deviceImport";
import { appInfo } from "../inApp";
import { ClientDownloads } from "./Apps";
import { gateNotice } from "../sync/model";
import { healthWarning } from "./CaptureHealth";
import { CaptureProblemsBadge } from "./CaptureProblems";
import { PhoneImport, ThisDevice } from "./DeviceImport";
import { SendDeviceNotes } from "./SendDeviceNotes";
import { errorText } from "./TypeEditor";

/** The commands every device app implements (D22). A device's own `devices/<platform>.wcl` may add more. */
export const BUILTIN_COMMANDS = ["send_sms", "upsert_contact", "delete_contact", "show_notification", "import_history"];

/** Whether the device file of `platform` declares `command`. */
export function declares(files: readonly DeviceFileInfo[] | undefined, platform: string, command: string): boolean {
  return (files ?? []).some((f) => f.platform === platform && (f.commands ?? []).some((c) => c.name === command));
}

/** The commands to offer as switches: the built-in ones, then any others the device already has enabled. */
export function commandChoices(enabled: readonly string[]): string[] {
  return [...BUILTIN_COMMANDS, ...enabled.filter((c) => !BUILTIN_COMMANDS.includes(c))];
}

const EXCLUSION_KINDS: { kind: DeviceExclusion["kind"]; label: string }[] = [
  { kind: "number", label: "Number" },
  { kind: "contact", label: "Contact" },
  { kind: "thread", label: "Thread" },
];

const PLATFORM_LABELS: Record<Device["platform"], string> = { linux: "Linux desktop", android: "Android" };

/** How many days a local vault moved aside by a fresh start is kept on the device (ticket 143). */
export const KEEP_PREVIOUS_DAYS = 7;

/**
 * What This device says after the app joined a different vault (ticket 143): it started a fresh
 * local copy, and where the old one went and for how long.
 */
export function freshStartText(f: FreshStart, at: (ms: number) => string = (ms) => new Date(ms).toLocaleString()): string {
  const folder = f.previous.split("/").pop() || f.previous;
  return (
    `This device joined a different vault on ${at(f.at_ms)}, so it started a fresh local copy and took this vault ` +
    `from the server; nothing of the other vault was sent here. The local vault it had before was moved to ` +
    `${folder} on the device, and is removed ${KEEP_PREVIOUS_DAYS} days after the move.`
  );
}

/**
 * Paired desktop and Android apps (D2, D3). Each device has a name, switches for what its scripts
 * may do, the commands server scripts may send it, and what it never captures (D22, D23, D25).
 * "Pair a device" shows a single-use code and its QR code for the app to scan. Above them, "Get
 * the apps" offers the builds the server hands out. Inside the Android app, "This device" imports
 * from the phone the app runs on; an Android phone's card asks that phone to import (D7).
 */
export function Devices() {
  const [devices, { refetch }] = createResource(api.devices);
  // In an app the list is the home server's, and changing a device needs the web app (D2).
  const [app] = createResource(appInfo);
  const inApp = () => (app.state === "ready" ? app() : null);
  // Which commands each platform's file declares; a vault with no files declares none.
  const [files] = createResource(() => api.deviceFiles().catch(() => [] as DeviceFileInfo[]));
  const [pairing, setPairing] = createSignal<PairCode | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  async function pair() {
    setError(null);
    setBusy(true);
    try {
      setPairing(await api.createPairCode(window.location.origin));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function donePairing() {
    setPairing(null);
    await refetch();
  }

  return (
    <section class="list-page devices-page">
      <header class="note-header">
        <h1>Devices</h1>
      </header>
      <Show when={inApp()?.fresh_start}>
        {(f) => (
          <p class="muted" role="note" data-testid="fresh-start">
            {freshStartText(f())}
          </p>
        )}
      </Show>
      <Show when={inAndroidApp()}>
        <ThisDevice />
      </Show>
      {/* The desktop app's This device holds only the send button; Android's has it beside the import. */}
      <Show when={inApp() && !inAndroidApp()}>
        <section class="field-card" aria-label="This device">
          <h2>This device</h2>
          <SendDeviceNotes />
        </section>
      </Show>
      <ClientDownloads />
      <h2>Paired devices</h2>
      <Show
        when={!inApp()}
        fallback={
          <p class="muted" role="note">
            Pair, rename and revoke devices, and change their settings, in the web app.
          </p>
        }
      >
        <p class="muted">
          The desktop and Android apps pair once with the password or a pairing code. Revoking a device stops its access
          at once.
        </p>
        <div class="actions">
          <button type="button" class="primary" disabled={busy()} onClick={() => void pair()}>
            Pair a device
          </button>
        </div>
      </Show>
      <Show when={pairing()}>
        {(p) => (
          <div class="pair-code" role="region" aria-label="Pair a device">
            <img
              class="pair-qr"
              alt="Pairing QR code"
              src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(p().qr_svg)}`}
            />
            <div>
              <p>Scan the code with the TheVault app, or type the code and server address into it.</p>
              <p class="pair-code-text" aria-label="Pairing code">
                {p().code}
              </p>
              <p>
                Server: <code>{p().server_url}</code>
              </p>
              <p class="muted">Works once, until {when(p().expires)}.</p>
              <button type="button" onClick={() => void donePairing()}>
                Done
              </button>
            </div>
          </div>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={devices.error}>
        <p class="error" role="alert">
          {errorText(devices.error)}
        </p>
      </Show>
      <Show when={devices.state === "ready" && devices()}>
        {(list) => (
          <Show when={list().length > 0} fallback={<p class="muted">No devices are paired.</p>}>
            <For each={list()}>
              {(d) => (
                <DeviceCard
                  device={d}
                  thisDevice={inApp()?.device_id === d.id}
                  readOnly={!!inApp()}
                  importDeclared={declares(files(), d.platform, "import_history")}
                  onChanged={() => void refetch()}
                />
              )}
            </For>
          </Show>
        )}
      </Show>
    </section>
  );
}

/**
 * One paired device. `thisDevice` marks the app the page runs in; `readOnly` shows the settings
 * without letting them change, in an app. An Android phone's card can ask the phone to import its
 * history; in an app only the app's own phone can be asked, through the home server.
 */
function DeviceCard(props: {
  device: Device;
  thisDevice?: boolean;
  readOnly?: boolean;
  importDeclared: boolean;
  onChanged: () => void;
}) {
  const d = props.device;
  const [name, setName] = createSignal(d.name);
  const [title, setTitle] = createSignal(d.name);
  const [contactsWrite, setContactsWrite] = createSignal(d.settings.contacts_write);
  const [smsSend, setSmsSend] = createSignal(d.settings.sms_send);
  const [commands, setCommands] = createSignal<string[]>([...d.settings.commands]);
  // What the server holds, which decides what the phone may be asked.
  const [savedCommands, setSavedCommands] = createSignal<string[]>([...d.settings.commands]);
  const [exclusions, setExclusions] = createSignal<DeviceExclusion[]>(d.settings.exclusions.map((e) => ({ ...e })));
  const [skipPattern, setSkipPattern] = createSignal(d.settings.skip_pattern);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const id = (part: string) => `device-${d.id}-${part}`;

  function toggleCommand(command: string, on: boolean) {
    setCommands((cs) => (on ? [...cs.filter((c) => c !== command), command] : cs.filter((c) => c !== command)));
  }

  function setExclusion(i: number, change: Partial<DeviceExclusion>) {
    setExclusions((list) => list.map((e, j) => (j === i ? { ...e, ...change } : e)));
  }

  async function save(ev: SubmitEvent) {
    ev.preventDefault();
    setError(null);
    setStatus(null);
    setBusy(true);
    try {
      const saved = await api.updateDevice(d.id, {
        name: name().trim(),
        settings: {
          commands: commands(),
          contacts_write: contactsWrite(),
          sms_send: smsSend(),
          exclusions: exclusions()
            .map((e) => ({ ...e, value: e.value.trim() }))
            .filter((e) => e.value !== ""),
          skip_pattern: skipPattern(),
        },
      });
      setTitle(saved.name);
      setSavedCommands([...saved.settings.commands]);
      setStatus("Saved.");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function revoke() {
    if (!confirm(`Revoke ${title()}? The app stops reaching the server at once and has to pair again.`)) return;
    setError(null);
    try {
      await api.revokeDevice(d.id);
      props.onChanged();
    } catch (e) {
      setError(errorText(e));
    }
  }

  return (
    <article class="field-card device-card" data-device={d.id} aria-label={`Device ${title()}`}>
      <header class="device-head">
        <h2>
          {title()}
          <Show when={props.thisDevice}>
            {" "}
            <span class="this-device">This device</span> <CaptureProblemsBadge />
          </Show>
        </h2>
        <span class="muted">
          {PLATFORM_LABELS[d.platform] ?? d.platform} · paired {when(d.created)} · last seen{" "}
          {d.last_seen === null ? "never" : when(d.last_seen)}
          {d.push ? " · push on" : ""}
        </span>
        <Show when={props.thisDevice}>
          <UpdateGateNotice />
        </Show>
        <Show when={healthWarning(d)}>
          {(w) => (
            <p class="device-health-warning" role="alert">
              {w()}
              <Show when={d.health?.reported}>{(at) => <span class="muted"> Reported {when(at())}.</span>}</Show>
            </p>
          )}
        </Show>
      </header>
      <form aria-label={`Settings for ${title()}`} onSubmit={save}>
        <fieldset class="device-settings" disabled={props.readOnly}>
        <div class="form-grid">
          <label for={id("name")}>Name</label>
          <input id={id("name")} value={name()} autocomplete="off" onInput={(e) => setName(e.currentTarget.value)} />
          <span>Scripts may</span>
          <div class="device-switches">
            <label>
              <input
                type="checkbox"
                checked={contactsWrite()}
                onChange={(e) => setContactsWrite(e.currentTarget.checked)}
              />{" "}
              Write phone contacts
            </label>
            <label>
              <input type="checkbox" checked={smsSend()} onChange={(e) => setSmsSend(e.currentTarget.checked)} /> Send
              SMS
            </label>
          </div>
          <span>Enabled commands</span>
          <div class="device-switches" role="group" aria-label="Enabled commands">
            <For each={commandChoices(d.settings.commands)}>
              {(c) => (
                <label>
                  <input
                    type="checkbox"
                    checked={commands().includes(c)}
                    onChange={(e) => toggleCommand(c, e.currentTarget.checked)}
                  />{" "}
                  <code>{c}</code>
                </label>
              )}
            </For>
          </div>
          <label for={id("skip")}>Skip pattern</label>
          <div>
            <textarea
              id={id("skip")}
              rows={2}
              spellcheck={false}
              value={skipPattern()}
              onInput={(e) => setSkipPattern(e.currentTarget.value)}
            />
            <p class="hint muted">A regular expression. An SMS whose text matches it is never captured.</p>
          </div>
        </div>
        <fieldset class="device-exclusions">
          <legend>Never capture</legend>
          <Show when={exclusions().length > 0} fallback={<p class="muted">Nothing is excluded.</p>}>
            <Index each={exclusions()}>
              {(e, i) => (
                <div class="device-exclusion" data-exclusion={i}>
                  <select
                    aria-label="Exclusion kind"
                    onChange={(ev) => setExclusion(i, { kind: ev.currentTarget.value as DeviceExclusion["kind"] })}
                  >
                    <For each={EXCLUSION_KINDS}>
                      {(k) => (
                        <option value={k.kind} selected={e().kind === k.kind}>
                          {k.label}
                        </option>
                      )}
                    </For>
                  </select>
                  <input
                    aria-label="Excluded value"
                    value={e().value}
                    placeholder={e().kind === "number" ? "+61400000000" : "id"}
                    onInput={(ev) => setExclusion(i, { value: ev.currentTarget.value })}
                  />
                  <button
                    type="button"
                    onClick={() => setExclusions((list) => list.filter((_, j) => j !== i))}
                    aria-label="Remove exclusion"
                  >
                    Remove
                  </button>
                </div>
              )}
            </Index>
          </Show>
          <button type="button" onClick={() => setExclusions((list) => [...list, { kind: "number", value: "" }])}>
            Add exclusion
          </button>
        </fieldset>
        </fieldset>
        <Show when={!props.readOnly}>
          <div class="actions">
            <button type="submit" class="primary" disabled={busy() || !name().trim()}>
              Save
            </button>
            <button type="button" class="danger" onClick={() => void revoke()}>
              Revoke
            </button>
          </div>
        </Show>
      </form>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={status()}>
        <p role="status">{status()}</p>
      </Show>
      <Show when={d.platform === "android"}>
        <PhoneImport
          device={d}
          declared={props.importDeclared}
          enabled={savedCommands().includes("import_history")}
          otherDeviceInApp={!!props.readOnly && !props.thisDevice}
        />
      </Show>
    </article>
  );
}

/**
 * This device's word on the version gate (D27): "Updating TheVault to X before syncing" while
 * sync waits for the app to update itself, or the warning that the server is newer and no
 * matching app build is published.
 */
function UpdateGateNotice() {
  const [sync] = createResource(() => api.appSync().catch(() => null));
  const notice = () => {
    const s = sync();
    return s ? gateNotice(s) : null;
  };
  return (
    <Show when={notice()}>
      {(text) => (
        <p class="device-update-gate" role="status">
          {text()}
        </p>
      )}
    </Show>
  );
}

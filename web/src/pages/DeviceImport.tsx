import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { type Device, type DeviceCommand, api } from "../api";
import {
  IMPORT_CHOICES,
  type ImportChoice,
  type ImportReport,
  RANGE_PRESETS,
  RCS_NOTE,
  type RangePreset,
  describeKind,
  importPayload,
  isImportReport,
  isoDay,
  missingPermissions,
  permissionName,
  thisPhone,
} from "../deviceImport";
import { errorText } from "./TypeEditor";

/** The command that asks a phone to import its history (D22). */
export const IMPORT_COMMAND = "import_history";

/** How often a running import is checked on. */
const POLL_MS = 1500;

function defaultChoice(): ImportChoice {
  const today = new Date();
  return {
    choices: ["messages", "calls"],
    preset: "30d",
    from: isoDay(new Date(today.getFullYear(), today.getMonth(), today.getDate() - 30)),
    to: isoDay(today),
  };
}

/** What to import and over which dates. */
function ImportChoices(props: { id: string; value: ImportChoice; onChange: (c: ImportChoice) => void; disabled?: boolean }) {
  const set = (change: Partial<ImportChoice>) => props.onChange({ ...props.value, ...change });
  const toggle = (id: string, on: boolean) =>
    set({ choices: on ? [...props.value.choices.filter((c) => c !== id), id] : props.value.choices.filter((c) => c !== id) });
  return (
    <div class="import-choices">
      <div class="device-switches" role="group" aria-label="What to import">
        <For each={IMPORT_CHOICES}>
          {(c) => (
            <label>
              <input
                type="checkbox"
                checked={props.value.choices.includes(c.id)}
                disabled={props.disabled}
                onChange={(e) => toggle(c.id, e.currentTarget.checked)}
              />{" "}
              {c.label}
            </label>
          )}
        </For>
      </div>
      <p class="hint muted">{RCS_NOTE}</p>
      <div class="form-grid">
        <label for={`${props.id}-range`}>Dates</label>
        <select
          id={`${props.id}-range`}
          disabled={props.disabled}
          onChange={(e) => set({ preset: e.currentTarget.value as RangePreset })}
        >
          <For each={RANGE_PRESETS}>
            {(p) => (
              <option value={p.id} selected={props.value.preset === p.id}>
                {p.label}
              </option>
            )}
          </For>
        </select>
        <Show when={props.value.preset === "custom"}>
          <label for={`${props.id}-from`}>From</label>
          <input
            id={`${props.id}-from`}
            type="date"
            value={props.value.from}
            disabled={props.disabled}
            onInput={(e) => set({ from: e.currentTarget.value })}
          />
          <label for={`${props.id}-to`}>To</label>
          <input
            id={`${props.id}-to`}
            type="date"
            value={props.value.to}
            disabled={props.disabled}
            onInput={(e) => set({ to: e.currentTarget.value })}
          />
        </Show>
      </div>
    </div>
  );
}

/** Each kind of a report, one line each. */
function ReportLines(props: { report: ImportReport }) {
  return (
    <ul class="import-report">
      <For each={props.report.kinds}>{(k) => <li data-kind={k.kind}>{describeKind(k)}</li>}</For>
    </ul>
  );
}

/** Missing permissions, said plainly, with the way to grant them on this phone. */
function MissingPermissions(props: { permissions: string[]; here: boolean }) {
  const [error, setError] = createSignal<string | null>(null);
  async function open() {
    setError(null);
    try {
      await thisPhone.openPermissions();
    } catch (e) {
      setError(errorText(e));
    }
  }
  return (
    <Show when={props.permissions.length > 0}>
      <div class="import-permissions" role="alert">
        <p>
          Android has not given TheVault {props.permissions.map(permissionName).join(", ")}, so that part was not read.{" "}
          {props.here
            ? "Allow it in the permission step, then import again."
            : "Open TheVault on the phone and allow it in Settings > Devices > This device, then import again."}
        </p>
        <Show when={props.here}>
          <button type="button" onClick={() => void open()}>
            Open the permission step
          </button>
        </Show>
        <Show when={error()}>
          <p class="error">{error()}</p>
        </Show>
      </div>
    </Show>
  );
}

/**
 * Settings > Devices "This device", inside the Android app: import messages, calls and contacts
 * from this phone over a range of dates, at any time, and check for new ones now. A message or
 * call that already has a note is left alone; one whose note was deleted gets a note again.
 */
export function ThisDevice() {
  const [choice, setChoice] = createSignal<ImportChoice>(defaultChoice());
  const [running, setRunning] = createSignal(false);
  const [report, setReport] = createSignal<ImportReport | null>(null);
  const [missing, setMissing] = createSignal<string[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [checked, setChecked] = createSignal<string | null>(null);
  const [checking, setChecking] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let section!: HTMLElement;
  onCleanup(() => clearTimeout(timer));

  async function poll() {
    try {
      const s = await thisPhone.importStatus();
      if (isImportReport(s.report)) setReport(s.report);
      if (s.error) setError(s.error);
      setRunning(s.running);
      if (s.running) {
        timer = setTimeout(() => void poll(), POLL_MS);
      } else {
        setMissing(missingPermissions(s.report));
      }
    } catch (e) {
      setRunning(false);
      setError(errorText(e));
    }
  }

  onMount(() => {
    if (window.location.hash === "#this-device") section.scrollIntoView();
    // An import started earlier may still run.
    void poll();
  });

  async function start() {
    setError(null);
    setReport(null);
    setMissing([]);
    const p = importPayload(choice());
    if (typeof p === "string") return setError(p);
    try {
      const answer = await thisPhone.importHistory(p);
      if (!answer.started) {
        setMissing(answer.missing.map((m) => m.permission));
        return;
      }
      setRunning(true);
      timer = setTimeout(() => void poll(), POLL_MS / 3);
    } catch (e) {
      setError(errorText(e));
    }
  }

  async function checkNow() {
    setChecked(null);
    setError(null);
    setChecking(true);
    try {
      const r = await thisPhone.checkNow();
      setChecked(r.captured === 1 ? "Found 1 new item." : `Found ${r.captured} new items.`);
      setMissing(r.missing);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setChecking(false);
    }
  }

  return (
    <section class="field-card this-device-import" id="this-device" ref={section} aria-label="This device">
      <h2>This device</h2>
      <p class="muted">
        Capture files new messages and calls as they arrive. To bring in what is already on the phone, or notes you
        deleted, import from the phone. Messages and calls that already have a note are left alone.
      </p>
      <ImportChoices id="this-device" value={choice()} onChange={setChoice} disabled={running()} />
      <div class="actions">
        <button type="button" class="primary" disabled={running()} onClick={() => void start()}>
          {running() ? "Importing…" : "Import from phone"}
        </button>
        <button type="button" disabled={checking()} onClick={() => void checkNow()}>
          Check for new messages and calls now
        </button>
      </div>
      <Show when={report()}>
        {(r) => (
          <div role="status" aria-label="Import progress">
            <p>{r().done ? "Import finished." : "Importing…"}</p>
            <ReportLines report={r()} />
          </div>
        )}
      </Show>
      <Show when={checked()}>
        <p role="status">{checked()}</p>
      </Show>
      <MissingPermissions permissions={missing()} here />
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </section>
  );
}

/** How a queued import_history stands, in words. */
function commandStatus(c: DeviceCommand): string {
  switch (c.status) {
    case "pending":
      return "Waiting for the phone. It runs the import when it next checks in.";
    case "delivered":
      return "The phone is importing…";
    case "done":
      return "Import finished.";
    case "failed":
      return "The import failed.";
    case "expired":
      return "The phone did not run the import in time.";
  }
}

/** The report a command carries: its progress while it runs, its result once done or failed. */
function commandReport(c: DeviceCommand): ImportReport | null {
  const r = c.result as { progress?: unknown } | null;
  if (isImportReport(r)) return r;
  if (r && isImportReport(r.progress)) return r.progress;
  return null;
}

/**
 * "Import from phone" on a phone's card: asks that phone, through the `import_history` command, to
 * import over the chosen kinds and dates, then shows how it goes. Disabled, saying why, until the
 * platform file declares the command and the phone has it enabled. Inside an app, whose device
 * token is not an admin's, only the app's own phone can be asked (D2).
 */
export function PhoneImport(props: { device: Device; declared: boolean; enabled: boolean; otherDeviceInApp?: boolean }) {
  const [choice, setChoice] = createSignal<ImportChoice>(defaultChoice());
  const [command, setCommand] = createSignal<DeviceCommand | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));
  const id = `device-${props.device.id}-import`;
  const blocked = () =>
    props.otherDeviceInApp
      ? "Ask another phone to import from the web app: an app may ask only its own phone."
      : !props.declared
      ? `The device file, devices/${props.device.platform}.wcl, does not declare the ${IMPORT_COMMAND} command. Add a command "${IMPORT_COMMAND}" block naming the device_import script to it.`
      : !props.enabled
        ? `Turn on ${IMPORT_COMMAND} under Enabled commands above and save to let this phone import.`
        : null;

  async function poll(cid: string) {
    try {
      const c = (await api.deviceCommands(props.device.id)).find((x) => x.id === cid);
      if (!c) return;
      setCommand(c);
      if (c.status === "pending" || c.status === "delivered") timer = setTimeout(() => void poll(cid), POLL_MS);
    } catch (e) {
      setError(errorText(e));
    }
  }

  async function start() {
    setError(null);
    const p = importPayload(choice());
    if (typeof p === "string") return setError(p);
    setBusy(true);
    try {
      const queued = await api.queueDeviceCommand(props.device.id, IMPORT_COMMAND, p);
      setCommand(queued);
      timer = setTimeout(() => void poll(queued.id), POLL_MS);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  const running = () => {
    const c = command();
    return c !== null && (c.status === "pending" || c.status === "delivered");
  };

  return (
    <fieldset class="device-import" aria-label="Import from phone">
      <legend>Import from phone</legend>
      <p class="muted">
        Ask this phone to import its messages, calls and contacts again. Messages and calls that already have a note are
        left alone; one whose note was deleted gets a note again.
      </p>
      <ImportChoices id={id} value={choice()} onChange={setChoice} disabled={blocked() !== null || running()} />
      <div class="actions">
        <button
          type="button"
          class="primary"
          disabled={blocked() !== null || busy() || running()}
          onClick={() => void start()}
        >
          Import from phone
        </button>
      </div>
      <Show when={blocked()}>
        <p class="hint muted" data-testid="import-blocked">
          {blocked()}
        </p>
      </Show>
      <Show when={command()}>
        {(c) => (
          <div role="status" aria-label="Import progress">
            <p>{commandStatus(c())}</p>
            <Show when={commandReport(c())}>{(r) => <ReportLines report={r()} />}</Show>
            <Show when={c().status === "failed" && typeof c().result === "string"}>
              <p class="error">{String(c().result)}</p>
            </Show>
            <MissingPermissions permissions={missingPermissions(commandReport(c()))} here={false} />
          </div>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </fieldset>
  );
}

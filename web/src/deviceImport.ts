// Importing a phone's messages, calls and contacts again (D7, D22): the choices Settings > Devices
// offers, the import_history payload they make, how a report reads, and, inside the Android app,
// the device plugin's commands for the phone the app runs on.

/** What an import can read. RCS is not among them: Google Messages keeps it to itself. */
export type ImportKind = "sms" | "mms" | "calls" | "contacts";

/** The choices on the page: SMS and MMS go together. */
export const IMPORT_CHOICES: { id: "messages" | "calls" | "contacts"; label: string; kinds: ImportKind[] }[] = [
  { id: "messages", label: "SMS and MMS messages", kinds: ["sms", "mms"] },
  { id: "calls", label: "Calls", kinds: ["calls"] },
  { id: "contacts", label: "Contacts", kinds: ["contacts"] },
];

export const RCS_NOTE =
  "RCS chats cannot be imported: Google Messages keeps them to itself. TheVault captures RCS messages from their notifications as they arrive.";

export type RangePreset = "7d" | "30d" | "1y" | "all" | "custom";

export const RANGE_PRESETS: { id: RangePreset; label: string }[] = [
  { id: "7d", label: "Last 7 days" },
  { id: "30d", label: "Last 30 days" },
  { id: "1y", label: "Last year" },
  { id: "all", label: "Everything" },
  { id: "custom", label: "Custom range" },
];

/** What the user chose: which kinds, and over which dates. `from` and `to` are `YYYY-MM-DD`, for a custom range. */
export interface ImportChoice {
  choices: string[];
  preset: RangePreset;
  from: string;
  to: string;
}

/** The import_history payload: `{ kinds, since, until }`, times RFC 3339 or null for no bound. */
export interface ImportPayload {
  kinds: ImportKind[];
  since: string | null;
  until: string | null;
}

/** A local `YYYY-MM-DD` as a Date at the start of that day, or null when it is not a date. */
function day(text: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text);
  if (!m) return null;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
}

/** `d` as `YYYY-MM-DD`, in local time. */
export function isoDay(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/**
 * The payload for `choice`, or a reason it cannot be sent. Presets count back from `now`; a custom
 * range runs from the start of its first day to the end of its last, in local time.
 */
export function importPayload(choice: ImportChoice, now: Date = new Date()): ImportPayload | string {
  const kinds = IMPORT_CHOICES.filter((c) => choice.choices.includes(c.id)).flatMap((c) => c.kinds);
  if (kinds.length === 0) return "Choose what to import.";
  const back = (days: number) => new Date(now.getTime() - days * 24 * 60 * 60 * 1000).toISOString();
  switch (choice.preset) {
    case "7d":
      return { kinds, since: back(7), until: null };
    case "30d":
      return { kinds, since: back(30), until: null };
    case "1y":
      return { kinds, since: back(365), until: null };
    case "all":
      return { kinds, since: null, until: null };
    case "custom": {
      const from = choice.from ? day(choice.from) : null;
      const to = choice.to ? day(choice.to) : null;
      if (choice.from && !from) return "The start date is not a date.";
      if (choice.to && !to) return "The end date is not a date.";
      const end = to ? new Date(to.getFullYear(), to.getMonth(), to.getDate() + 1).getTime() - 1 : null;
      if (from && end !== null && from.getTime() > end) return "The start date is after the end date.";
      return {
        kinds,
        since: from ? from.toISOString() : null,
        until: end !== null ? new Date(end).toISOString() : null,
      };
    }
  }
}

/** How one kind of an import went, as the core reports it. */
export interface ImportKindReport {
  kind: ImportKind;
  found: number;
  imported: number;
  notes: number;
  existing: number;
  skipped: number;
  failed: number;
  missing_permission: string | null;
  error: string | null;
  done: boolean;
}

export interface ImportReport {
  kinds: ImportKindReport[];
  since: number | null;
  until: number | null;
  done: boolean;
  message?: string;
}

const KIND_LABELS: Record<ImportKind, string> = { sms: "SMS", mms: "MMS", calls: "Calls", contacts: "Contacts" };

/** A permission's short name: `android.permission.READ_SMS` as `READ_SMS`. */
export function permissionName(p: string): string {
  return p.replace(/^android\.permission\./, "");
}

/** One line for a kind of a report. */
export function describeKind(k: ImportKindReport): string {
  const label = KIND_LABELS[k.kind] ?? k.kind;
  if (k.missing_permission) return `${label}: not read. Android has not given TheVault ${permissionName(k.missing_permission)}.`;
  if (k.found === 0 && k.error) return `${label}: not read. ${k.error}`;
  const parts = [`${k.found} found`, `${k.notes} new ${k.notes === 1 ? "note" : "notes"}`];
  if (k.existing > 0) parts.push(`${k.existing} already in the vault`);
  if (k.skipped > 0) parts.push(`${k.skipped} excluded`);
  if (k.failed > 0) parts.push(`${k.failed} failed`);
  return `${label}: ${parts.join(", ")}${k.done ? "" : "…"}`;
}

/** The permissions a report says are missing. */
export function missingPermissions(report: ImportReport | null | undefined): string[] {
  return [...new Set((report?.kinds ?? []).flatMap((k) => (k.missing_permission ? [k.missing_permission] : [])))];
}

/** Whether `value` looks like an import report. */
export function isImportReport(value: unknown): value is ImportReport {
  return typeof value === "object" && value !== null && Array.isArray((value as ImportReport).kinds);
}

// ---- the phone the Android app runs on ------------------------------------------------------

interface PluginIpc {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
}

/** The device plugin's IPC, inside the Android app only. */
function pluginIpc(): PluginIpc | null {
  const w = globalThis as { __thevaultApiOverIpc?: boolean; __TAURI_INTERNALS__?: PluginIpc };
  return w.__thevaultApiOverIpc && w.__TAURI_INTERNALS__ ? w.__TAURI_INTERNALS__ : null;
}

/** Whether this page runs inside the Android app, on the phone it can import from. */
export function inAndroidApp(): boolean {
  return pluginIpc() !== null;
}

function plugin<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const ipc = pluginIpc();
  if (!ipc) return Promise.reject(new Error("only the Android app can do this"));
  return ipc.invoke<T>(`plugin:vault-device|${command}`, args);
}

export interface ImportStatus {
  running: boolean;
  report?: ImportReport;
  error?: string;
}

/** The system screens a capture health problem's button opens. */
export type CaptureFix = "app_details" | "notification_access" | "battery" | "capture_setup";

/** A capture kind's state: working, permission missing, blocked by restricted settings, or not set up. */
export type CaptureState = "working" | "permission_missing" | "restricted" | "not_set_up";

/** The phone's capture health, as the device plugin's `capture_health` answers it (ticket 91). */
export interface CaptureHealthStatus {
  paired: boolean;
  setup_done: boolean;
  kinds: { kind: string; label: string; state: CaptureState; message: string; fix: CaptureFix | null; then?: CaptureFix }[];
  battery: { optimised: boolean; samsung: boolean; message: string; fix: CaptureFix | null; samsung_line?: string };
  background: { work: string; label: string; last: number | null; expected: boolean; stale: boolean }[];
  problems: string[];
  checked: number;
}

/** The device plugin's commands for this phone. */
export const thisPhone = {
  /** Start an import in the background. Nothing starts when a chosen kind's permission is missing. */
  importHistory: (p: ImportPayload) =>
    plugin<{ started: boolean; missing: { kind: ImportKind; permission: string }[] }>("import_history", { ...p }),
  importStatus: () => plugin<ImportStatus>("import_status"),
  /** Read every store from where capture stopped, now. */
  checkNow: () => plugin<{ captured: number; missing: string[] }>("check_now"),
  /** The capture setup screen: each permission in turn. */
  openPermissions: () => plugin<void>("open_capture_setup"),
  /** The phone's capture health: each capture kind, battery optimisation and background work (ticket 91). */
  captureHealth: () => plugin<CaptureHealthStatus>("capture_health"),
  /** Open the system screen that fixes a capture health problem. */
  openCaptureFix: (fix: CaptureFix) => plugin<void>("open_capture_fix", { fix }),
  /** What the app update check last decided, why, and when (ms since the epoch); null before the first. */
  updateStatus: () => plugin<{ status: string | null; message: string | null; checked: number | null }>("update_status"),
  /** Run the app update check now, once the phone is online. */
  checkForUpdates: () => plugin<void>("check_for_updates"),
};

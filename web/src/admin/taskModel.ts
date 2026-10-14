// A scheduled task as the task editor holds it, and its task file (docs/vault-format.md "Task
// files", D10). Items left at their documented default are not written.

import type { Task } from "../api";
import { wclField } from "../defs/wcl";

export interface TaskModel {
  label: string;
  description: string;
  schedule: string;
  timezone: string;
  script: string;
  function: string;
  /** Hosts the script may reach over HTTP. `["*"]` is any host. */
  allow_domains: string[];
  secrets: string[];
  /** Absolute directories the script may read with `fs`. */
  read_paths: string[];
  /** Absolute directories the script may read and write with `fs`. */
  write_paths: string[];
  retries: number;
  /** ISO 8601 durations, as the file holds them. */
  retry_delay: string;
  timeout: string;
  paused: boolean;
}

export const TASK_DEFAULTS = {
  timezone: "UTC",
  function: "run",
  retries: 3,
  retry_delay: "PT1M",
  timeout: "PT30S",
} as const;

export function emptyTask(): TaskModel {
  return {
    label: "",
    description: "",
    schedule: "",
    timezone: TASK_DEFAULTS.timezone,
    script: "",
    function: "",
    allow_domains: [],
    secrets: [],
    read_paths: [],
    write_paths: [],
    retries: TASK_DEFAULTS.retries,
    retry_delay: TASK_DEFAULTS.retry_delay,
    timeout: TASK_DEFAULTS.timeout,
    paused: false,
  };
}

/** Milliseconds as the shortest ISO 8601 duration that says them exactly. */
export function isoDuration(ms: number): string {
  if (ms % 3_600_000 === 0) return `PT${ms / 3_600_000}H`;
  if (ms % 60_000 === 0) return `PT${ms / 60_000}M`;
  return `PT${ms / 1000}S`;
}

/** The editor's model of a task the API describes. `label` is left empty when it is the name. */
export function taskFromApi(t: Task): TaskModel {
  return {
    label: t.label && t.label !== t.name ? t.label : "",
    description: t.description ?? "",
    schedule: t.schedule ?? "",
    timezone: t.timezone ?? TASK_DEFAULTS.timezone,
    script: t.script ?? "",
    function: t.function && t.function !== TASK_DEFAULTS.function ? t.function : "",
    allow_domains: [...(t.allow_domains ?? [])],
    secrets: [...(t.secrets ?? [])],
    read_paths: [...(t.read_paths ?? [])],
    write_paths: [...(t.write_paths ?? [])],
    retries: t.retries ?? TASK_DEFAULTS.retries,
    retry_delay: t.retry_delay_ms === undefined ? TASK_DEFAULTS.retry_delay : isoDuration(t.retry_delay_ms),
    timeout: t.timeout_ms === undefined ? TASK_DEFAULTS.timeout : isoDuration(t.timeout_ms),
    paused: t.paused ?? false,
  };
}

/** The task file for `m`. */
export function taskWcl(m: TaskModel): string {
  const opt = (name: string, v: string, dflt = "") => (v.trim() && v.trim() !== dflt ? wclField(name, v.trim()) : "");
  return [
    opt("label", m.label),
    opt("description", m.description),
    wclField("schedule", m.schedule.trim()),
    opt("timezone", m.timezone, TASK_DEFAULTS.timezone),
    wclField("script", m.script),
    opt("function", m.function, TASK_DEFAULTS.function),
    m.allow_domains.length ? wclField("allow_domains", m.allow_domains) : "",
    m.secrets.length ? wclField("secrets", m.secrets) : "",
    m.read_paths.length ? wclField("read_paths", m.read_paths) : "",
    m.write_paths.length ? wclField("write_paths", m.write_paths) : "",
    m.retries !== TASK_DEFAULTS.retries ? wclField("retries", m.retries) : "",
    opt("retry_delay", m.retry_delay, TASK_DEFAULTS.retry_delay),
    opt("timeout", m.timeout, TASK_DEFAULTS.timeout),
    m.paused ? wclField("paused", true) : "",
  ].join("");
}

/** What is wrong with the model before the server sees it. Empty when nothing is. */
export function taskProblems(m: TaskModel): string[] {
  const out: string[] = [];
  if (m.schedule.trim().split(/\s+/).filter(Boolean).length !== 5) {
    out.push("The schedule needs five cron fields: minute, hour, day of month, month, day of week.");
  }
  if (!m.script) out.push("Choose the script the task runs.");
  for (const p of [...m.read_paths, ...m.write_paths]) {
    if (!p.startsWith("/")) out.push(`The directory ${JSON.stringify(p)} needs an absolute path, starting with /.`);
  }
  if (!Number.isInteger(m.retries) || m.retries < 0) out.push("Retries must be a whole number, 0 or more.");
  return out;
}

/** One entry per non-blank line of `text`. */
export function lines(text: string): string[] {
  return text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
}

/** How the editor offers web access: none, a list of hosts, or any host (`"*"`). */
export type WebAccess = "none" | "hosts" | "any";

export function webAccess(domains: string[]): WebAccess {
  if (domains.includes("*")) return "any";
  return domains.length ? "hosts" : "none";
}

/** One directory a task may use, as the editor lists them. */
export interface DirAccess {
  path: string;
  write: boolean;
}

export function dirAccess(m: Pick<TaskModel, "read_paths" | "write_paths">): DirAccess[] {
  return [
    ...m.read_paths.map((path) => ({ path, write: false })),
    ...m.write_paths.map((path) => ({ path, write: true })),
  ];
}

/** The `read_paths` and `write_paths` of the editor's directory rows. Blank rows are dropped. */
export function splitDirs(dirs: DirAccess[]): { read_paths: string[]; write_paths: string[] } {
  const keep = dirs.map((d) => ({ ...d, path: d.path.trim() })).filter((d) => d.path);
  return {
    read_paths: keep.filter((d) => !d.write).map((d) => d.path),
    write_paths: keep.filter((d) => d.write).map((d) => d.path),
  };
}

/** The schedule presets the editor offers. `custom` is any other cron expression. */
export type Preset = "custom" | "hourly" | "daily" | "weekdays" | "weekly";

export interface PresetChoice {
  preset: Preset;
  /** `HH:MM`, for the presets that run at a time of day. */
  time: string;
  /** Day of the week, 0 for Sunday, for `weekly`. */
  day: number;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** The preset a cron expression is, with its time and day, or `custom`. */
export function presetOf(cron: string): PresetChoice {
  const f = cron.trim().split(/\s+/);
  const base: PresetChoice = { preset: "custom", time: "07:00", day: 1 };
  if (f.length !== 5) return base;
  const [mi, h, dom, mon, dow] = f as [string, string, string, string, string];
  if (mi === "0" && h === "*" && dom === "*" && mon === "*" && dow === "*") return { ...base, preset: "hourly" };
  if (!/^\d{1,2}$/.test(mi) || !/^\d{1,2}$/.test(h) || dom !== "*" || mon !== "*") return base;
  const time = `${pad(Number(h))}:${pad(Number(mi))}`;
  if (Number(h) > 23 || Number(mi) > 59) return base;
  if (dow === "*") return { preset: "daily", time, day: 1 };
  if (dow === "1-5") return { preset: "weekdays", time, day: 1 };
  if (/^[0-6]$/.test(dow)) return { preset: "weekly", time, day: Number(dow) };
  return base;
}

/** The cron expression of a preset choice. `custom` keeps `current`. */
export function cronOf(c: PresetChoice, current: string): string {
  const [h, m] = c.time.split(":").map((x) => Number.parseInt(x, 10));
  const hh = Number.isInteger(h) ? h : 7;
  const mm = Number.isInteger(m) ? m : 0;
  switch (c.preset) {
    case "hourly":
      return "0 * * * *";
    case "daily":
      return `${mm} ${hh} * * *`;
    case "weekdays":
      return `${mm} ${hh} * * 1-5`;
    case "weekly":
      return `${mm} ${hh} * * ${c.day}`;
    default:
      return current;
  }
}

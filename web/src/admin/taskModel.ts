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
  allow_domains: string[];
  secrets: string[];
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

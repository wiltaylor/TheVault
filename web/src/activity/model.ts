// What the Log page (activity Y4, Y5) works out from activity entries, kept apart from the page so
// it can be tested alone: the categories and their icons, who an actor is, where an entry's
// subject lives, whether a live entry passes the filters, the day groups, and the tool calls of an
// AI turn nested under it.

import type { ActivityFilter, ActivityItem, Json } from "../api";

/** The category chips, in order. All is the empty category. */
export const CATEGORIES: { id: string; label: string; icon: string }[] = [
  { id: "", label: "All", icon: "list" },
  { id: "ai", label: "AI", icon: "bot" },
  { id: "notes", label: "Notes", icon: "file-text" },
  { id: "definitions", label: "Definitions", icon: "shapes" },
  { id: "automation", label: "Automation", icon: "clock" },
  { id: "outbox", label: "Outbox", icon: "send" },
  { id: "sync", label: "Sync", icon: "refresh-cw" },
  { id: "auth", label: "Auth", icon: "key-round" },
];

/** The icon of category `id`. */
export function categoryIcon(id: string): string {
  return CATEGORIES.find((c) => c.id === id)?.icon ?? "circle";
}

/** The label of category `id`. */
export function categoryLabel(id: string): string {
  return CATEGORIES.find((c) => c.id === id)?.label ?? id;
}

/** The session the owner signs in with, as the server names it: "You" in the actor picker. */
export const YOU = "session:owner";

/** The page's address showing agent `name`'s AI activity (activity Y5). */
export function logHref(agent: string): string {
  return `/log?category=ai&actor=agent:${encodeURIComponent(agent)}`;
}

/** The actor of `item` as the `actor` filter names it, `<kind>:<name>`. */
export function actorKey(item: Pick<ActivityItem, "actor_kind" | "actor">): string {
  return `${item.actor_kind}:${item.actor}`;
}

/** `<kind>:<name>` split at its first colon. */
export function splitActor(key: string): { kind: string; name: string } {
  const i = key.indexOf(":");
  return i < 0 ? { kind: key, name: "" } : { kind: key.slice(0, i), name: key.slice(i + 1) };
}

/** Who actor `kind` `name` is, in words. `agentLabel` names an agent the sidebar knows. */
export function actorLabel(kind: string, name: string, agentLabel?: (name: string) => string | undefined): string {
  switch (kind) {
    case "session":
      return "You";
    case "agent":
      return agentLabel?.(name) ?? name;
    case "handler":
      return `Script ${name}`;
    case "task":
      return `Task ${name}`;
    case "token":
      return `Token ${name}`;
    case "device":
      return name || "A device";
    case "server":
      return "Server";
    default:
      return name ? `${kind} ${name}` : kind;
  }
}

/** What an entry is about: its kind (`note`, `thread`, ...) and id, or null. */
export function subjectOf(item: Pick<ActivityItem, "subject_kind" | "subject">): { kind: string; id: string } | null {
  const raw = item.subject ?? "";
  if (!raw) return null;
  let kind = item.subject_kind ?? "";
  let id = raw;
  const i = raw.indexOf(":");
  // The subject may carry its kind, as `note:<id>`.
  if (i > 0 && (!kind || raw.slice(0, i) === kind)) {
    kind = raw.slice(0, i);
    id = raw.slice(i + 1);
  }
  return kind && id ? { kind, id } : null;
}

function str(v: Json | undefined): string | null {
  return typeof v === "string" && v ? v : null;
}

/** The thread an AI entry belongs to, if any. */
export function threadOf(item: ActivityItem): string | null {
  const s = subjectOf(item);
  if (s?.kind === "thread") return s.id;
  return str(item.detail?.thread);
}

/** The agent an entry is by or about, if any. */
function agentOf(item: ActivityItem): string | null {
  if (item.actor_kind === "agent") return item.actor;
  return str(item.detail?.agent);
}

/** Where an entry's subject opens, and what the link says; null when it links nowhere. */
export function subjectLink(item: ActivityItem): { href: string; label: string } | null {
  const s = subjectOf(item);
  if (!s) return null;
  const id = encodeURIComponent(s.id);
  const title = str(item.detail?.title);
  switch (s.kind) {
    case "note":
      return { href: `/notes/${id}`, label: title ?? "Open note" };
    case "thread": {
      const agent = agentOf(item);
      return agent
        ? { href: `/agents/${encodeURIComponent(agent)}?thread=${id}`, label: "Open thread" }
        : { href: `/notes/${id}`, label: "Open thread" };
    }
    case "type":
      return { href: `/types/${id}`, label: `Type ${s.id}` };
    case "taxonomy":
      return { href: `/settings/taxonomies/${id}`, label: `Taxonomy ${s.id}` };
    case "agent":
      return { href: `/agents/${id}`, label: `Agent ${s.id}` };
    case "task_run": {
      const task = str(item.detail?.task);
      return { href: task ? `/settings/scheduled/${encodeURIComponent(task)}` : "/settings/scheduled", label: "Open task run" };
    }
    case "outbox":
      return { href: `/outbox?item=${id}`, label: "Open in Outbox" };
    case "device":
      return { href: "/settings/devices", label: "Open devices" };
    default:
      return null;
  }
}

/** Whether `item` passes `filter`, so a live entry belongs on the page. */
export function matches(item: ActivityItem, filter: ActivityFilter): boolean {
  if (filter.category && item.category !== filter.category) return false;
  if (filter.actor) {
    const want = splitActor(filter.actor);
    if (item.actor_kind !== want.kind) return false;
    if (want.name && item.actor !== want.name) return false;
  }
  if (filter.kind && item.kind !== filter.kind && !item.kind.startsWith(`${filter.kind}.`)) return false;
  if (filter.q && !item.summary.toLowerCase().includes(filter.q.toLowerCase())) return false;
  return true;
}

/** `a` and `b` as one list, newest first, each entry once (ids are ULIDs, so they sort by time). */
export function mergeItems(a: ActivityItem[], b: ActivityItem[]): ActivityItem[] {
  const byId = new Map<string, ActivityItem>();
  for (const it of a) byId.set(it.id, it);
  for (const it of b) byId.set(it.id, it);
  return [...byId.values()].sort((x, y) => (x.id < y.id ? 1 : x.id > y.id ? -1 : 0));
}

/** An entry with the entries nested under it: an AI turn with its tool calls. */
export interface LogRow {
  item: ActivityItem;
  children: ActivityItem[];
}

/**
 * `items` (newest first) as rows, each AI turn's start with the entries of that turn nested under
 * it, oldest first, when both are on the page. A turn is matched by its thread and `detail.turn`
 * when the entries carry one, and otherwise by order: an entry of a thread belongs to the turn of
 * that thread that started last before it, until that turn ends.
 */
export function nestTurns(items: ActivityItem[]): LogRow[] {
  const rows = new Map<string, LogRow>();
  const nested = new Set<string>();
  const byTurn = new Map<string, LogRow>();
  const open = new Map<string, LogRow>();
  for (const item of [...items].reverse()) {
    const row: LogRow = { item, children: [] };
    rows.set(item.id, row);
    if (item.category !== "ai") continue;
    const thread = threadOf(item);
    if (!thread) continue;
    const turn = item.detail?.turn;
    const turnKey = turn === undefined || turn === null ? null : `${thread}#${String(turn)}`;
    if (item.kind === "ai.turn.start") {
      if (turnKey) byTurn.set(turnKey, row);
      open.set(thread, row);
      continue;
    }
    const head = turnKey ? byTurn.get(turnKey) : open.get(thread);
    if (head) {
      head.children.push(item);
      nested.add(item.id);
    }
    if (item.kind === "ai.turn.end" && open.get(thread) === head) open.delete(thread);
  }
  return items.filter((it) => !nested.has(it.id)).map((it) => rows.get(it.id)!);
}

/** The local day of `ms`, as a key. */
export function dayKey(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** The heading of the day of `ms`: Today, Yesterday, or the date. */
export function dayLabel(ms: number, now = Date.now()): string {
  const key = dayKey(ms);
  if (key === dayKey(now)) return "Today";
  const y = new Date(now);
  y.setDate(y.getDate() - 1);
  if (key === dayKey(y.getTime())) return "Yesterday";
  return new Date(ms).toLocaleDateString(undefined, { weekday: "long", year: "numeric", month: "long", day: "numeric" });
}

/** `rows` (newest first) grouped by local day. */
export function byDay(rows: LogRow[], now = Date.now()): { key: string; label: string; rows: LogRow[] }[] {
  const days: { key: string; label: string; rows: LogRow[] }[] = [];
  for (const row of rows) {
    const key = dayKey(row.item.at_ms);
    const last = days[days.length - 1];
    if (last?.key === key) last.rows.push(row);
    else days.push({ key, label: dayLabel(row.item.at_ms, now), rows: [row] });
  }
  return days;
}

/** The time of day of `ms`, as each entry shows it. */
export function timeOf(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

/** A detail value as text. */
export function detailText(v: Json): string {
  if (typeof v === "string") return v;
  return JSON.stringify(v);
}

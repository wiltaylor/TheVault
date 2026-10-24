// The Log, `/log` (activity Y4, Y5): what happened in the vault, newest first and grouped by day,
// with a toolbar of category chips, an actor picker, a search box and a pause toggle. Each entry
// shows its time, who did it, its category, its one-line summary and a link to what it is about;
// expanding it shows its detail. An AI turn's tool calls nest under the turn when both are on the
// page. Older pages load as the list scrolls. New entries arrive on the live event stream
// (`activity`); the apps, which poll, read the newest page again every 10 seconds instead. The
// filters live in the address, so `/log?category=ai&actor=agent:<name>` (Show in log on an agent's
// screen) opens one agent's AI activity. The log needs `admin`: an app whose device token lacks it
// says to open the log in the web app.

import { useSearchParams } from "@solidjs/router";
import { For, Show, createEffect, createMemo, createResource, createSignal, on, onCleanup, onMount } from "solid-js";
import { agentLabel, agents } from "../agents";
import { type ActivityFilter, type ActivityItem, ApiError, api } from "../api";
import { onActivity, onActivityResync } from "../activity/live";
import {
  CATEGORIES,
  YOU,
  actorKey,
  actorLabel,
  byDay,
  categoryIcon,
  categoryLabel,
  detailText,
  matches,
  mergeItems,
  nestTurns,
  splitActor,
  subjectLink,
  timeOf,
} from "../activity/model";
import { AGENT_ICON } from "../agents";
import { TypeIcon } from "../components/Icon";
import { appInfo } from "../inApp";
import { errorText } from "./TypeEditor";

/** Entries per page. */
export const LOG_PAGE = 100;

/** How often an app reads the newest page again while the Log is open (activity Y3). */
export const APP_POLL_MS = 10_000;

/** How long the search box waits after typing before it searches. */
export const SEARCH_DELAY_MS = 250;

const one = (v: string | string[] | undefined): string => (Array.isArray(v) ? (v[0] ?? "") : (v ?? ""));

function labelOfAgent(name: string): string | undefined {
  const a = agents().find((x) => x.name === name);
  return a ? agentLabel(a) : undefined;
}

function iconOfAgent(name: string): string {
  return agents().find((x) => x.name === name)?.icon || AGENT_ICON;
}

/** One actor the picker offers. */
interface ActorOption {
  key: string;
  label: string;
}

export function Log() {
  const [params, setParams] = useSearchParams();
  const filter = createMemo<ActivityFilter>(() => ({
    category: one(params.category) || null,
    actor: one(params.actor) || null,
    kind: one(params.kind) || null,
    q: one(params.q) || null,
  }));
  const [items, setItems] = createSignal<ActivityItem[]>([]);
  const [next, setNext] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [loaded, setLoaded] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [webOnly, setWebOnly] = createSignal(false);
  const [paused, setPaused] = createSignal(false);
  /** Entries that came while paused, shown when the list is resumed. */
  const [held, setHeld] = createSignal<ActivityItem[]>([]);
  const [open, setOpen] = createSignal<Set<string>>(new Set());
  const [search, setSearch] = createSignal(one(params.q));
  let loads = 0;

  const [app] = createResource(appInfo);

  function failed(e: unknown) {
    if (e instanceof ApiError && (e.needsWebApp || (e.status === 403 && app()))) setWebOnly(true);
    else setError(errorText(e));
  }

  /** Read the newest page for the filters, replacing the list. */
  async function load() {
    const mine = ++loads;
    const f = filter();
    setLoading(true);
    setError(null);
    try {
      const page = await api.activity({ ...f, limit: LOG_PAGE });
      if (mine !== loads) return;
      setItems(page.items);
      setNext(page.next);
      setHeld([]);
      setWebOnly(false);
    } catch (e) {
      if (mine === loads) failed(e);
    } finally {
      if (mine === loads) {
        setLoading(false);
        setLoaded(true);
      }
    }
  }

  /** Read the newest page again and add what is new, keeping the older pages loaded. */
  async function refreshNewest() {
    const mine = loads;
    try {
      const page = await api.activity({ ...filter(), limit: LOG_PAGE });
      if (mine !== loads) return;
      if (paused()) {
        const have = new Set(items().map((i) => i.id));
        setHeld((h) => mergeItems(h, page.items.filter((i) => !have.has(i.id))));
      } else setItems((cur) => mergeItems(cur, page.items));
      if (!items().length) setNext(page.next);
    } catch {
      // The next read tries again; the list stays as it was.
    }
  }

  /** Read the page before the oldest entry shown. */
  async function loadOlder() {
    const before = next();
    if (!before || loading()) return;
    const mine = loads;
    setLoading(true);
    try {
      const page = await api.activity({ ...filter(), before, limit: LOG_PAGE });
      if (mine !== loads) return;
      setItems((cur) => mergeItems(cur, page.items));
      setNext(page.next);
    } catch (e) {
      if (mine === loads) failed(e);
    } finally {
      if (mine === loads) setLoading(false);
    }
  }

  createEffect(
    on(
      () => JSON.stringify(filter()),
      () => void load(),
    ),
  );

  // Live entries from the event stream.
  onCleanup(
    onActivity((item) => {
      if (!matches(item, filter())) return;
      if (paused()) setHeld((h) => mergeItems(h, [item]));
      else setItems((cur) => mergeItems(cur, [item]));
    }),
  );
  onCleanup(onActivityResync(() => void refreshNewest()));

  // The apps poll: read the newest page every 10 seconds while the page is open.
  createEffect(() => {
    if (!app()) return;
    const timer = setInterval(() => {
      if (!webOnly()) void refreshNewest();
    }, APP_POLL_MS);
    onCleanup(() => clearInterval(timer));
  });

  function setPause(p: boolean) {
    setPaused(p);
    if (!p && held().length) {
      setItems((cur) => mergeItems(cur, held()));
      setHeld([]);
    }
  }

  // The search box applies after a pause in typing, or at once on Enter.
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(searchTimer));
  function applySearch(q: string) {
    clearTimeout(searchTimer);
    if (q.trim() !== one(params.q)) setParams({ q: q.trim() || undefined }, { replace: true });
  }
  createEffect(on(() => one(params.q), (q) => {
    if (q !== search().trim()) setSearch(q);
  }, { defer: true }));

  // Who the actor picker offers: you, each agent, each script and task, each device, and anyone
  // else on the page.
  const [scripts] = createResource(() => api.scripts().catch(() => [] as string[]));
  const [tasks] = createResource(() => api.tasks().catch(() => []));
  const [devices] = createResource(() => api.devices().catch(() => []));
  const actorGroups = createMemo(() => {
    const groups: { label: string; options: ActorOption[] }[] = [
      { label: "People", options: [{ key: YOU, label: "You" }] },
      { label: "Agents", options: agents().map((a) => ({ key: `agent:${a.name}`, label: agentLabel(a) })) },
      { label: "Scripts", options: (scripts() ?? []).map((s) => ({ key: `handler:${s}`, label: `Script ${s}` })) },
      { label: "Tasks", options: (tasks() ?? []).map((t) => ({ key: `task:${t.name}`, label: `Task ${t.name}` })) },
      { label: "Devices", options: (devices() ?? []).map((d) => ({ key: `device:${d.name}`, label: d.name })) },
    ];
    const known = new Set(groups.flatMap((g) => g.options.map((o) => o.key)));
    const others: ActorOption[] = [];
    const add = (key: string) => {
      if (!key || known.has(key)) return;
      known.add(key);
      const { kind, name } = splitActor(key);
      others.push({ key, label: actorLabel(kind, name, labelOfAgent) });
    };
    add(filter().actor ?? "");
    for (const it of items()) add(actorKey(it));
    if (others.length) groups.push({ label: "Others", options: others });
    return groups.filter((g) => g.options.length);
  });

  const days = createMemo(() => byDay(nestTurns(items())));

  const toggle = (id: string) =>
    setOpen((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  // Infinite scroll: load the next older page as the end of the list comes into view.
  let sentinel: HTMLDivElement | undefined;
  onMount(() => {
    if (typeof IntersectionObserver === "undefined" || !sentinel) return;
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) void loadOlder();
    });
    io.observe(sentinel);
    onCleanup(() => io.disconnect());
  });

  const heading = () => {
    const f = filter();
    if (f.category !== "ai") return null;
    if (!f.actor) return "AI activity";
    const { kind, name } = splitActor(f.actor);
    return `AI activity of ${actorLabel(kind, name, labelOfAgent)}`;
  };

  function Entry(p: { item: ActivityItem; nested?: ActivityItem[] }) {
    const it = () => p.item;
    const link = () => subjectLink(it());
    const expanded = () => open().has(it().id);
    const detail = () => Object.entries(it().detail ?? {});
    return (
      <li class="log-entry" classList={{ "log-turn": !!p.nested?.length }} data-kind={it().kind}>
        <div class="log-row">
          <button
            type="button"
            class="log-toggle"
            aria-expanded={expanded()}
            aria-label={`${it().summary}, ${timeOf(it().at_ms)}`}
            onClick={() => toggle(it().id)}
          >
            <time class="log-time" datetime={new Date(it().at_ms).toISOString()}>
              {timeOf(it().at_ms)}
            </time>
            <span class="log-actor" data-actor={actorKey(it())}>
              <Show when={it().actor_kind === "agent"}>
                <TypeIcon name={iconOfAgent(it().actor)} size={14} />
              </Show>
              {actorLabel(it().actor_kind, it().actor, labelOfAgent)}
            </span>
            <span class="log-category" title={categoryLabel(it().category)}>
              <TypeIcon name={categoryIcon(it().category)} size={14} />
              <span class="visually-hidden">{categoryLabel(it().category)}</span>
            </span>
            <span class="log-summary">{it().summary}</span>
          </button>
          <Show when={link()}>
            {(l) => (
              <a class="log-subject" href={l().href}>
                {l().label}
              </a>
            )}
          </Show>
        </div>
        <Show when={expanded()}>
          <dl class="log-detail">
            <dt>Kind</dt>
            <dd>{it().kind}</dd>
            <dt>When</dt>
            <dd>{new Date(it().at_ms).toLocaleString()}</dd>
            <dt>Actor</dt>
            <dd>{actorKey(it())}</dd>
            <Show when={it().subject}>
              <dt>Subject</dt>
              <dd>{it().subject}</dd>
            </Show>
            <For each={detail()}>
              {([k, v]) => (
                <>
                  <dt>{k}</dt>
                  <dd>{detailText(v)}</dd>
                </>
              )}
            </For>
          </dl>
        </Show>
        <Show when={p.nested?.length}>
          <ul class="log-children" aria-label="In this turn">
            <For each={p.nested}>{(c) => <Entry item={c} />}</For>
          </ul>
        </Show>
      </li>
    );
  }

  return (
    <section class="list-page log-page">
      <header class="note-header">
        <h1>Log</h1>
      </header>
      <Show
        when={!webOnly()}
        fallback={
          <p role="status">Open the log in the web app. This app's link to your server cannot read the vault's activity.</p>
        }
      >
        <div class="log-toolbar">
          <div class="log-chips" role="group" aria-label="Category">
            <For each={CATEGORIES}>
              {(c) => (
                <button
                  type="button"
                  class="log-chip"
                  aria-pressed={(filter().category ?? "") === c.id}
                  onClick={() => setParams({ category: c.id || undefined }, { replace: true })}
                >
                  {c.label}
                </button>
              )}
            </For>
          </div>
          <select
            class="log-actor-picker"
            aria-label="Actor"
            value={filter().actor ?? ""}
            onChange={(e) => setParams({ actor: e.currentTarget.value || undefined }, { replace: true })}
          >
            <option value="">Everyone</option>
            <For each={actorGroups()}>
              {(g) => (
                <optgroup label={g.label}>
                  <For each={g.options}>{(o) => <option value={o.key} selected={o.key === (filter().actor ?? "")}>{o.label}</option>}</For>
                </optgroup>
              )}
            </For>
          </select>
          <input
            type="search"
            class="log-search"
            aria-label="Search the log"
            placeholder="Search the log"
            value={search()}
            onInput={(e) => {
              const q = e.currentTarget.value;
              setSearch(q);
              clearTimeout(searchTimer);
              searchTimer = setTimeout(() => applySearch(q), SEARCH_DELAY_MS);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") applySearch(e.currentTarget.value);
            }}
          />
          <button type="button" class="log-pause" aria-pressed={paused()} onClick={() => setPause(!paused())}>
            <TypeIcon name={paused() ? "play" : "pause"} size={16} /> {paused() ? "Resume" : "Pause"}
          </button>
        </div>
        <Show when={heading()}>
          <p class="log-preset muted">{heading()}</p>
        </Show>
        <Show when={paused() && held().length}>
          <button type="button" class="log-held" onClick={() => setPause(false)}>
            {held().length === 1 ? "1 new entry" : `${held().length} new entries`}: show
          </button>
        </Show>
        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>
        <Show when={loaded() && !loading() && !error() && items().length === 0}>
          <p class="muted">Nothing in the log matches.</p>
        </Show>
        <For each={days()}>
          {(day) => (
            <section class="log-day" aria-label={day.label}>
              <h2 class="log-day-heading">{day.label}</h2>
              <ul class="log-list">
                <For each={day.rows}>{(row) => <Entry item={row.item} nested={row.children} />}</For>
              </ul>
            </section>
          )}
        </For>
        <div ref={sentinel} class="log-end">
          <Show when={next()}>
            <button type="button" disabled={loading()} onClick={() => void loadOlder()}>
              Load older
            </button>
          </Show>
        </div>
      </Show>
    </section>
  );
}

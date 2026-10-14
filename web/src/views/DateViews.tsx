import { Calendar } from "@fullcalendar/core";
import dayGridPlugin from "@fullcalendar/daygrid";
import timeGridPlugin from "@fullcalendar/timegrid";
import { useNavigate } from "@solidjs/router";
import { Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { Timeline } from "vis-timeline/standalone";
import "vis-timeline/styles/vis-timeline-graph2d.min.css";
import { type Row, api } from "../api";
import { changes } from "../changes";
import { types } from "../session";
import { openInBackground, wantsNewTab } from "../workspace/events";
import type { ViewDef } from "./model";
import { type TimelineItem, calendarEvent, dateField, hasDate, narrow, timelineItem, visibleWindow } from "./narrow";

/** Rows per request, the API's largest page. */
const PAGE_SIZE = 1000;

/** Most pages one window loads. A window holding more shows the first of them and says so. */
const MAX_PAGES = 10;

const CALENDAR_VIEWS: Record<string, string> = { month: "dayGridMonth", week: "timeGridWeek", day: "timeGridDay" };

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Every row of `query`, a page at a time, up to MAX_PAGES. `capped` says whether rows were left. */
async function loadAll(query: string, thisId: string | undefined): Promise<{ rows: Row[]; capped: boolean }> {
  const rows: Row[] = [];
  let cursor: string | null = null;
  for (let n = 0; n < MAX_PAGES; n++) {
    const page = await api.query(query, cursor, PAGE_SIZE, thisId);
    rows.push(...page.rows);
    cursor = page.next;
    if (!cursor) return { rows, capped: false };
  }
  return { rows, capped: true };
}

function Problem(props: { text: string }) {
  return (
    <p class="error" role="alert" data-testid="view-problem">
      {props.text}
    </p>
  );
}

function CappedNote(props: { capped: boolean }) {
  return (
    <Show when={props.capped}>
      <p class="muted">
        More than {MAX_PAGES * PAGE_SIZE} notes fall in this range. Showing the first {MAX_PAGES * PAGE_SIZE}.
      </p>
    </Show>
  );
}

/**
 * Notes on the days of a date, datetime or date range field, with FullCalendar's MIT core (D29).
 * Only the notes in the range on screen are loaded: the view's query is narrowed to it.
 */
/** Below this width of its own the calendar switches to its compact layout. */
export const COMPACT_CALENDAR = 520;

/**
 * The toolbar and header options for a calendar `width` pixels wide. A narrow one keeps the
 * arrows, title and today on top and moves the view buttons to a row underneath, with one-letter
 * weekday headers, so nothing runs past its edge.
 */
export function calendarLayout(width: number) {
  const views = "dayGridMonth,timeGridWeek,timeGridDay";
  return width < COMPACT_CALENDAR
    ? {
        compact: true,
        headerToolbar: { left: "prev,next", center: "title", right: "today" },
        footerToolbar: { center: views },
        dayHeaderFormat: { weekday: "narrow" as const },
      }
    : {
        compact: false,
        headerToolbar: { left: "prev,next today", center: "title", right: views },
        footerToolbar: false as const,
        dayHeaderFormat: { weekday: "short" as const },
      };
}

export function CalendarView(props: { view: ViewDef; query: string; thisId: string | undefined }) {
  const check = createMemo(() => dateField(types(), props.query, props.view.date));
  return (
    <Show when={check().field} fallback={<Problem text={check().problem ?? ""} />}>
      {(f) => {
        let box!: HTMLDivElement;
        const navigate = useNavigate();
        const [error, setError] = createSignal<string | null>(null);
        const [capped, setCapped] = createSignal(false);
        let calendar: Calendar | undefined;
        onMount(() => {
          const start = calendarLayout(box.clientWidth || COMPACT_CALENDAR);
          box.classList.toggle("fc-compact", start.compact);
          calendar = new Calendar(box, {
            plugins: [dayGridPlugin, timeGridPlugin],
            initialView: CALENDAR_VIEWS[props.view.mode] ?? "dayGridMonth",
            headerToolbar: start.headerToolbar,
            footerToolbar: start.footerToolbar,
            dayHeaderFormat: start.dayHeaderFormat,
            height: "auto",
            dayMaxEvents: true,
            events: (info, success, failure) => {
              const query = narrow(props.query, visibleWindow(f(), info.start, info.end));
              loadAll(query, props.thisId).then(
                ({ rows, capped }) => {
                  setError(null);
                  setCapped(capped);
                  success(rows.flatMap((r) => calendarEvent(r, f()) ?? []).map((e) => ({ ...e, url: `/notes/${e.id}` })));
                },
                (e) => {
                  setError(message(e));
                  failure(e instanceof Error ? e : new Error(String(e)));
                },
              );
            },
            eventClick: (info) => {
              info.jsEvent.preventDefault();
              // FullCalendar renders an event as a link; Ctrl/Cmd+click opens it in a new tab.
              if (wantsNewTab(info.jsEvent)) openInBackground(`/notes/${info.event.id}`);
              else navigate(`/notes/${info.event.id}`);
            },
            eventDidMount: (info) => {
              info.el.dataset.testid = "calendar-event";
              info.el.dataset.id = info.event.id;
            },
          });
          calendar.render();
          // Follow the pane's width, not the window's: switch layouts as it crosses the threshold.
          let compact = start.compact;
          const observer =
            typeof ResizeObserver === "undefined"
              ? null
              : new ResizeObserver(() => {
                  const next = calendarLayout(box.clientWidth);
                  if (next.compact === compact || !calendar || box.clientWidth === 0) return;
                  compact = next.compact;
                  box.classList.toggle("fc-compact", compact);
                  calendar.setOption("headerToolbar", next.headerToolbar);
                  calendar.setOption("footerToolbar", next.footerToolbar);
                  calendar.setOption("dayHeaderFormat", next.dayHeaderFormat);
                });
          observer?.observe(box);
          onCleanup(() => {
            observer?.disconnect();
            calendar?.destroy();
          });
        });
        createEffect(
          on(
            () => [props.query, props.thisId, f(), changes()],
            () => calendar?.refetchEvents(),
            { defer: true },
          ),
        );
        createEffect(
          on(
            () => props.view.mode,
            (mode) => calendar?.changeView(CALENDAR_VIEWS[mode] ?? "dayGridMonth"),
            { defer: true },
          ),
        );
        return (
          <div class="query-calendar">
            <Show when={error()}>
              <Problem text={error()!} />
            </Show>
            <CappedNote capped={capped()} />
            <div ref={box} data-testid="calendar" />
          </div>
        );
      }}
    </Show>
  );
}

/**
 * Notes along a date, datetime or date range field with vis-timeline (D29): a range for a date
 * range, a point for a single date. It opens on the first page of dated notes, then loads the
 * notes in whatever range is scrolled or zoomed to.
 */
export function TimelineView(props: { view: ViewDef; query: string; thisId: string | undefined }) {
  const check = createMemo(() => dateField(types(), props.query, props.view.date));
  return (
    <Show when={check().field} fallback={<Problem text={check().problem ?? ""} />}>
      {(f) => {
        let box!: HTMLDivElement;
        const navigate = useNavigate();
        const [error, setError] = createSignal<string | null>(null);
        const [capped, setCapped] = createSignal(false);
        const [empty, setEmpty] = createSignal(false);
        const items = new Map<string, TimelineItem>();
        let timeline: Timeline | undefined;
        let generation = 0;

        const add = (rows: Row[]) => {
          for (const r of rows) {
            const item = timelineItem(r, f());
            if (item) items.set(item.id, item);
            else items.delete(r.id);
          }
          timeline?.setItems([...items.values()]);
        };

        async function loadWindow(start: Date, end: Date) {
          const gen = generation;
          try {
            const { rows, capped } = await loadAll(narrow(props.query, visibleWindow(f(), start, end)), props.thisId);
            if (gen !== generation) return;
            setCapped(capped);
            add(rows);
          } catch (e) {
            if (gen === generation) setError(message(e));
          }
        }

        /** Load the range on screen again and show only what it holds, keeping the view where it is. */
        async function reloadWindow() {
          if (!timeline) return;
          const gen = ++generation;
          const { start, end } = timeline.getWindow();
          try {
            const { rows, capped } = await loadAll(narrow(props.query, visibleWindow(f(), start, end)), props.thisId);
            if (gen !== generation) return;
            setError(null);
            setCapped(capped);
            items.clear();
            add(rows);
          } catch (e) {
            if (gen === generation) setError(message(e));
          }
        }

        async function reset() {
          const gen = ++generation;
          items.clear();
          timeline?.setItems([]);
          setError(null);
          setEmpty(false);
          try {
            const page = await api.query(narrow(props.query, hasDate(f())), null, PAGE_SIZE, props.thisId);
            if (gen !== generation) return;
            add(page.rows);
            setEmpty(page.total === 0);
            if (items.size > 0) timeline?.fit({ animation: false });
          } catch (e) {
            if (gen === generation) setError(message(e));
          }
        }

        onMount(() => {
          timeline = new Timeline(box, [], {
            stack: true,
            zoomKey: "ctrlKey",
            orientation: "top",
            height: "24rem",
          });
          timeline.on("rangechanged", (p: { start: Date; end: Date; byUser: boolean }) => {
            if (p.byUser) void loadWindow(p.start, p.end);
          });
          timeline.on("click", (p: { item: string | null; what: string | null }) => {
            if (p.item && p.what === "item") navigate(`/notes/${p.item}`);
          });
          onCleanup(() => {
            generation++;
            timeline?.destroy();
          });
          void reset();
        });
        createEffect(
          on(
            () => [props.query, props.thisId, f()],
            () => void reset(),
            { defer: true },
          ),
        );
        createEffect(on(changes, () => void reloadWindow(), { defer: true }));
        return (
          <div class="query-timeline">
            <Show when={error()}>
              <Problem text={error()!} />
            </Show>
            <Show when={empty()}>
              <p class="muted">No notes in this query have a {f().label.toLowerCase()}.</p>
            </Show>
            <CappedNote capped={capped()} />
            <div ref={box} data-testid="timeline" />
          </div>
        );
      }}
    </Show>
  );
}

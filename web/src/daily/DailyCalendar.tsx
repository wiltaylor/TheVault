// The month calendar at the top of the right sidebar beside the daily notes page or a daily note.
// Each day links to its daily note, so a click opens it in the active pane and a middle-click or
// Ctrl/Cmd+click a background tab, as any link does. Days that have a note carry a dot.

import { For, type JSX, Show, createEffect, createMemo, createResource, createSignal, on, onCleanup } from "solid-js";
import { api } from "../api";
import { changes, onNoteChangedElsewhere } from "../changes";
import { rightIsDrawer, setRightDrawer } from "../sidebars";
import { activePane } from "../workspace/model";
import { navigatePane, workspace } from "../workspace/store";
import {
  type Month,
  addDays,
  addMonths,
  dailyHref,
  dailyQuery,
  dayLabel,
  dayOf,
  gridRange,
  monthGrid,
  monthOf,
  monthTitle,
  sameMonth,
  shiftMonth,
  todayDay,
  weekStartOf,
  weekdayNames,
  weekdayOf,
} from "./calendar";
import { isoWeek, plannerHref, periodTitle } from "../planner/period";
import "./calendar.css";

export const CALENDAR_KEY = "thevault.dailyCalendar";

function loadOpen(): boolean {
  try {
    const raw = localStorage.getItem(CALENDAR_KEY);
    const v = raw ? (JSON.parse(raw) as { open?: unknown }) : null;
    return typeof v?.open === "boolean" ? v.open : true;
  } catch {
    return true;
  }
}

const [calendarOpen, setOpenSignal] = createSignal(loadOpen());

/** Expand or collapse the calendar; remembered per browser. */
export function setCalendarOpen(open: boolean) {
  setOpenSignal(open);
  try {
    localStorage.setItem(CALENDAR_KEY, JSON.stringify({ open }));
  } catch {
    // Private browsing: the choice lasts until the page closes.
  }
}

/**
 * The calendar section. `day` is the day the active pane shows; `noteId` its note, if it has one
 * yet, so creating it brings its dot. `children` go at the end of the header (the button that
 * hides the sidebar).
 */
export function DailyCalendar(props: { day: string; noteId: string; shown: boolean; children?: JSX.Element }) {
  const locale = typeof navigator !== "undefined" ? navigator.language || "en-AU" : "en-AU";
  const weekStart = weekStartOf(locale);
  const names = weekdayNames(locale, weekStart);

  const [today, setToday] = createSignal(todayDay());
  const tick = setInterval(() => setToday(todayDay()), 60_000);
  onCleanup(() => clearInterval(tick));

  const [view, setView] = createSignal<Month>(monthOf(props.day), { equals: sameMonth });
  // The day that takes the keyboard focus in the grid: the shown day, else where the arrows moved.
  const [focusDay, setFocusDay] = createSignal(props.day);
  createEffect(
    on(
      () => props.day,
      (d) => {
        setView(monthOf(d));
        setFocusDay(d);
      },
      { defer: true },
    ),
  );

  const weeks = createMemo(() => monthGrid(view(), weekStart));

  // Which days of the grid have a daily note. Asked again after this client writes a note, when
  // one changes elsewhere, and when the shown day or its note changes (opening today makes it).
  const [elsewhere, setElsewhere] = createSignal(0);
  onCleanup(onNoteChangedElsewhere(() => setElsewhere((n) => n + 1)));
  const key = createMemo(() => {
    if (!props.shown || !calendarOpen()) return null;
    const { first, last } = gridRange(view(), weekStart);
    return { first, last, w: changes(), e: elsewhere(), id: props.noteId, day: props.day };
  }, null, {
    equals: (a, b) =>
      a === b ||
      (!!a && !!b && a.first === b.first && a.last === b.last && a.w === b.w && a.e === b.e && a.id === b.id && a.day === b.day),
  });
  const [noted] = createResource(key, async ({ first, last }) => {
    const page = await api.query(dailyQuery(first, last), null, 100);
    return new Set(page.rows.map((r) => String(r.fields.date ?? "").slice(0, 10)));
  });
  const hasNote = (day: string) => (noted.latest ?? noted())?.has(day) ?? false;

  let grid: HTMLTableElement | undefined;

  function moveFocus(day: string) {
    setFocusDay(day);
    if (!sameMonth(monthOf(day), view())) setView(monthOf(day));
    queueMicrotask(() => grid?.querySelector<HTMLElement>(`[data-day="${day}"]`)?.focus());
  }

  function stepMonth(n: number) {
    setView(shiftMonth(view(), n));
    setFocusDay(addMonths(focusDay(), n));
    // Keep the focused day in the month shown.
    if (!sameMonth(monthOf(focusDay()), view())) setFocusDay(dayOf(view().year, view().month, 1));
  }

  function goToday() {
    const t = today();
    setView(monthOf(t));
    setFocusDay(t);
  }

  function openHref(href: string) {
    navigatePane(activePane(workspace()), href);
    if (rightIsDrawer()) setRightDrawer(false);
  }

  function open(day: string) {
    openHref(dailyHref(day, today()));
  }

  /** A plain left click opens `href` in the active pane; any other click is the link's own. */
  const follow = (href: () => string) => (e: MouseEvent) => {
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
    e.preventDefault();
    openHref(href());
  };
  const monthHref = () => plannerHref("month", dayOf(view().year, view().month, 1), today());
  /** The Monday of a grid week, whichever day the week starts on. */
  const mondayOf = (week: { day: string }[]) => week.find((c) => weekdayOf(c.day) === 1)!.day;

  function onKey(e: KeyboardEvent) {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const d = focusDay();
    const weekPos = (weekdayOf(d) - weekStart + 7) % 7;
    let next: string | null = null;
    switch (e.key) {
      case "ArrowLeft":
        next = addDays(d, -1);
        break;
      case "ArrowRight":
        next = addDays(d, 1);
        break;
      case "ArrowUp":
        next = addDays(d, -7);
        break;
      case "ArrowDown":
        next = addDays(d, 7);
        break;
      case "Home":
        next = addDays(d, -weekPos);
        break;
      case "End":
        next = addDays(d, 6 - weekPos);
        break;
      case "PageUp":
        next = addMonths(d, e.shiftKey ? -12 : -1);
        break;
      case "PageDown":
        next = addMonths(d, e.shiftKey ? 12 : 1);
        break;
      case " ":
        e.preventDefault();
        open(d);
        return;
      default:
        return;
    }
    e.preventDefault();
    moveFocus(next);
  }

  const titleId = "daily-calendar-title";
  return (
    <section class="daily-calendar" aria-label="Calendar">
      <div class="sidebar-head graph-head">
        <button
          type="button"
          class="graph-toggle"
          aria-expanded={calendarOpen()}
          aria-controls="daily-calendar-body"
          title={calendarOpen() ? "Collapse the calendar" : "Expand the calendar"}
          onClick={() => setCalendarOpen(!calendarOpen())}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m9 6 6 6-6 6" />
          </svg>
          Calendar
        </button>
        {props.children}
      </div>
      <Show when={calendarOpen()}>
        <div id="daily-calendar-body" class="calendar-body">
          <div class="calendar-bar">
            <button type="button" class="calendar-step" aria-label="Previous month" title="Previous month" onClick={() => stepMonth(-1)}>
              ‹
            </button>
            <h3 class="calendar-title" id={titleId} aria-live="polite">
              <a
                href={monthHref()}
                class="calendar-month-link"
                data-testid="calendar-month-link"
                title={`Open the note for ${periodTitle("month", dayOf(view().year, view().month, 1))}`}
                on:click={follow(monthHref)}
              >
                {monthTitle(view(), locale)}
              </a>
            </h3>
            <button type="button" class="calendar-step" aria-label="Next month" title="Next month" onClick={() => stepMonth(1)}>
              ›
            </button>
            <button
              type="button"
              class="calendar-today"
              disabled={sameMonth(view(), monthOf(today())) && focusDay() === today()}
              onClick={goToday}
            >
              Today
            </button>
          </div>
          <table class="calendar-grid" role="grid" aria-labelledby={titleId} ref={grid} onKeyDown={onKey}>
            <thead>
              <tr>
                <td class="calendar-week-head" aria-hidden="true" />
                <For each={names}>
                  {(n) => (
                    <th scope="col" abbr={n.long}>
                      <span aria-hidden="true">{n.short.slice(0, 2)}</span>
                      <span class="visually-hidden">{n.long}</span>
                    </th>
                  )}
                </For>
              </tr>
            </thead>
            <tbody>
              <For each={weeks()}>
                {(week) => (
                  <tr>
                    <th scope="row" class="calendar-week">
                      <a
                        href={plannerHref("week", mondayOf(week), today())}
                        data-week={mondayOf(week)}
                        tabindex={-1}
                        aria-label={periodTitle("week", mondayOf(week))}
                        title={`Open the note for ${periodTitle("week", mondayOf(week))}`}
                        on:click={follow(() => plannerHref("week", mondayOf(week), today()))}
                      >
                        {isoWeek(mondayOf(week)).week}
                      </a>
                    </th>
                    <For each={week}>
                      {(cell) => (
                        <td
                          role="gridcell"
                          aria-selected={cell.day === props.day}
                          classList={{
                            outside: !cell.inMonth,
                            today: cell.day === today(),
                            selected: cell.day === props.day,
                          }}
                        >
                          <a
                            href={dailyHref(cell.day, today())}
                            data-day={cell.day}
                            tabindex={cell.day === focusDay() ? 0 : -1}
                            aria-current={cell.day === today() ? "date" : undefined}
                            aria-label={`${dayLabel(cell.day, locale)}${hasNote(cell.day) ? ", has a note" : ""}`}
                            onFocus={() => setFocusDay(cell.day)}
                            on:click={(e: MouseEvent) => {
                              if (e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
                              e.preventDefault();
                              open(cell.day);
                            }}
                          >
                            {cell.date}
                            <Show when={hasNote(cell.day)}>
                              <span class="calendar-dot" aria-hidden="true" />
                            </Show>
                          </a>
                        </td>
                      )}
                    </For>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </section>
  );
}

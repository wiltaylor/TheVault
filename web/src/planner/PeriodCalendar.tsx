// A `period_calendar` block in a planner note's template (docs/vault-format.md,
// "period_calendar"): the notes of its query, events by default, on the days of the note's week,
// month, quarter or year. Every day links to its daily note; every event opens on click. Links
// are plain in-app links, so a click opens them in this pane and a middle or Ctrl/Cmd+click in a
// background tab, as any link does.

import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup, onMount, useContext } from "solid-js";
import { type Block, type Row, api } from "../api";
import { changes } from "../changes";
import { FieldRefs } from "../components/fieldRefs";
import { monthGrid, monthOf, todayDay } from "../daily/calendar";
import { types } from "../session";
import { calendarEvent, dateField, narrow, visibleWindow, type CalendarEvent } from "../views/narrow";
import { useThisNote } from "../views/thisNote";
import { MONTHS, DAY, longDay, periodEnd, periodStart, periodTitle, plannerHref, shiftDay, shiftPeriod } from "./period";
import { type CalendarPeriod, eventTime, eventsByDay, readPeriodCalendar, sortDay } from "./periodCalendar";
import { type TimedSegment, allDayBars, clock, hourRange, scrollHour, timedSegments } from "./weekGrid";
import "./planner.css";

const PAGE_SIZE = 1000;
const MAX_PAGES = 10;
const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/** Every row of `query`, a page at a time, up to MAX_PAGES. */
async function loadAll(query: string, thisId: string | undefined): Promise<Row[]> {
  const rows: Row[] = [];
  let cursor: string | null = null;
  for (let n = 0; n < MAX_PAGES; n++) {
    const page = await api.query(query, cursor, PAGE_SIZE, thisId);
    rows.push(...page.rows);
    cursor = page.next;
    if (!cursor) break;
  }
  return rows;
}

function local(day: string): Date {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(y!, m! - 1, d!);
}

/** The block. The note's own `date` field places the period; `date` overrides it, for tests. */
export function PeriodCalendar(props: { block: Block; date?: string }) {
  const refs = useContext(FieldRefs);
  const thisId = useThisNote();
  const def = createMemo(() => readPeriodCalendar(props.block));
  const noteDate = () => {
    const d = props.date ?? refs?.text("date") ?? "";
    return DAY.test(d.slice(0, 10)) ? d.slice(0, 10) : "";
  };
  return (
    <Show
      when={def()}
      fallback={<p class="muted block-placeholder">A period calendar needs a period: week, month, quarter or year.</p>}
    >
      {(d) => (
        <Show when={noteDate()} fallback={<p class="muted block-placeholder">The calendar of this note's {d().period} shows here once it has a date.</p>}>
          {(day) => <PeriodCalendarBody period={d().period} query={d().query} dateName={d().date} day={day()} thisId={thisId} />}
        </Show>
      )}
    </Show>
  );
}

function PeriodCalendarBody(props: { period: CalendarPeriod; query: string; dateName: string; day: string; thisId: string | undefined }) {
  const first = () => periodStart(props.period, props.day);
  const last = () => periodEnd(props.period, props.day);
  const check = createMemo(() => dateField(types(), props.query, props.dateName));
  const [rows] = createResource(
    () => {
      const f = check().field;
      if (!f) return null;
      return { f, query: props.query, first: first(), last: last(), thisId: props.thisId, w: changes() };
    },
    async (k) => {
      const q = narrow(k.query, visibleWindow(k.f, local(k.first), local(shiftDay(k.last, 1))));
      const all = await loadAll(q, k.thisId);
      return all.flatMap((r) => calendarEvent(r, k.f) ?? []);
    },
  );
  const events = createMemo(() => (rows.error ? [] : (rows.latest ?? [])));
  const byDay = createMemo(() => eventsByDay(events(), first(), last()));
  const eventsOn = (day: string) => sortDay(byDay().get(day) ?? []);
  const today = todayDay();

  return (
    <div role="group" class={`period-calendar period-${props.period}`} aria-label={`Calendar of ${periodTitle(props.period, first())}`} data-testid="period-calendar">
      <Show when={check().problem}>
        <p class="error" role="alert" data-testid="view-problem">
          {check().problem}
        </p>
      </Show>
      <Show when={rows.error}>
        <p class="error" role="alert">
          {String(rows.error instanceof Error ? rows.error.message : rows.error)}
        </p>
      </Show>
      {/* A query that fails, or a field it cannot place notes by, shows the problem in place of the calendar. */}
      <Show when={!check().problem && !rows.error}>
      <Show when={props.period === "week"}>
        <WeekCalendar first={first()} today={today} events={events()} ready={rows.state === "ready"} eventsOn={eventsOn} />
      </Show>
      <Show when={props.period === "month" || props.period === "quarter"}>
        <div class="period-months">
          <For each={Array.from({ length: props.period === "month" ? 1 : 3 }, (_, i) => shiftPeriod("month", first(), i))}>
            {(month) => <MonthGrid month={month} today={today} eventsOn={eventsOn} title={props.period === "quarter"} />}
          </For>
        </div>
      </Show>
      <Show when={props.period === "year"}>
        <YearGrid first={first()} today={today} eventsOn={eventsOn} />
      </Show>
      </Show>
    </div>
  );
}

/** Below this width a week lists its days one under another rather than in seven columns. */
export const NARROW_WEEK = 560;

/**
 * A week: a time grid of seven equal day columns where there is room, else its days one under
 * another. A ResizeObserver picks between them, so only one is ever in the page.
 */
function WeekCalendar(props: {
  first: string;
  today: string;
  events: CalendarEvent[];
  ready: boolean;
  eventsOn: (day: string) => CalendarEvent[];
}) {
  let box!: HTMLDivElement;
  const [width, setWidth] = createSignal(0);
  onMount(() => {
    setWidth(box.clientWidth);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setWidth(box.clientWidth));
    ro.observe(box);
    onCleanup(() => ro.disconnect());
  });
  const days = () => Array.from({ length: 7 }, (_, i) => shiftDay(props.first, i));
  // An unmeasured box, as in a test without layout, shows the grid.
  const narrowPane = () => width() > 0 && width() < NARROW_WEEK;
  return (
    <div ref={box} class="period-week-box">
      <Show
        when={!narrowPane()}
        fallback={
          <div class="period-week" data-testid="period-week-list">
            <For each={days()}>
              {(day, i) => (
                <div class="period-week-day" classList={{ today: day === props.today }} data-day={day}>
                  <a class="period-day-link" href={plannerHref("day", day, props.today)} aria-label={longDay(day)} data-testid="period-day">
                    <span class="period-weekday">{WEEKDAYS[i()]}</span>
                    <span class="period-date">{local(day).getDate()}</span>
                  </a>
                  <EventList events={props.eventsOn(day)} day={day} />
                </div>
              )}
            </For>
          </div>
        }
      >
        <WeekGrid days={days()} today={props.today} events={props.events} ready={props.ready} />
      </Show>
    </div>
  );
}

/** Minutes since local midnight, now, kept up to date to the minute. */
function useNowMinutes() {
  const read = () => {
    const d = new Date();
    return d.getHours() * 60 + d.getMinutes();
  };
  const [now, setNow] = createSignal(read());
  const timer = setInterval(() => setNow(read()), 60_000);
  onCleanup(() => clearInterval(timer));
  return now;
}

/**
 * The time grid: day headers and the all-day row above, hour rows down the left, each timed
 * event a block placed by its start and sized by its length, overlapping ones side by side.
 */
function WeekGrid(props: { days: string[]; today: string; events: CalendarEvent[]; ready: boolean }) {
  let scroller!: HTMLDivElement;
  let body!: HTMLDivElement;
  const segments = createMemo(() => timedSegments(props.events, props.days[0]!, props.days[6]!));
  const all = () => [...segments().values()].flat();
  const hours = createMemo(() => hourRange(all()));
  const bars = createMemo(() => allDayBars(props.events, props.days[0]!));
  const allDayRows = () => bars().reduce((n, b) => Math.max(n, b.row + 1), 0);
  const now = useNowMinutes();
  const hourList = () => Array.from({ length: hours().last - hours().first }, (_, i) => hours().first + i);
  /** A time's offset down the grid, in hour rows. */
  const rows = (minutes: number) => (minutes - hours().first * 60) / 60;

  // Open at the week's earliest event, or 08:00, once its events are in; later refreshes keep
  // wherever the grid has been scrolled to.
  let scrolled = false;
  createEffect(() => {
    if (scrolled || !props.ready) return;
    scrolled = true;
    const perHour = body.offsetHeight / Math.max(1, hours().last - hours().first);
    scroller.scrollTop = Math.max(0, (scrollHour(all()) - hours().first) * perHour);
  });

  const style = (s: TimedSegment) => ({
    top: `calc(var(--hour-height) * ${rows(s.start)})`,
    height: `calc(var(--hour-height) * ${(s.end - s.start) / 60} - 2px)`,
    left: `calc(${(s.lane / s.lanes) * 100}% + 1px)`,
    width: `calc(${100 / s.lanes}% - 3px)`,
  });

  return (
    <div class="week-grid" data-testid="period-week-grid" style={{ "--hours": hours().last - hours().first }}>
      <div class="week-scroll" ref={scroller}>
        <div class="week-head" style={{ "--allday-rows": allDayRows() }}>
          <div class="week-corner" />
          <For each={props.days}>
            {(day, i) => (
              <div class="week-head-day" classList={{ today: day === props.today }} data-day={day} style={{ "grid-column": i() + 2 }}>
                <a class="period-day-link" href={plannerHref("day", day, props.today)} aria-label={longDay(day)} data-testid="period-day">
                  <span class="period-weekday">{WEEKDAYS[i()]}</span>
                  <span class="period-date">{local(day).getDate()}</span>
                </a>
              </div>
            )}
          </For>
          <Show when={bars().length}>
            <div class="week-allday-label" style={{ "grid-row": `2 / span ${allDayRows()}` }}>
              All day
            </div>
            <For each={bars()}>
              {(b) => (
                <a
                  class="period-event all-day week-allday"
                  classList={{ "continues-before": b.continuesBefore, "continues-after": b.continuesAfter }}
                  style={{ "grid-column": `${b.column + 2} / span ${b.span}`, "grid-row": b.row + 2 }}
                  href={`/notes/${b.event.id}`}
                  data-testid="period-event"
                  data-id={b.event.id}
                  data-days={b.days.join(" ")}
                  title={b.event.title}
                >
                  <span class="period-event-title">{b.event.title}</span>
                </a>
              )}
            </For>
          </Show>
        </div>
        <div class="week-body" ref={body}>
          <div class="week-gutter" aria-hidden="true">
            <For each={hourList()}>
              {(h) => (
                <span class="week-hour" style={{ top: `calc(var(--hour-height) * ${h - hours().first})` }}>
                  {clock(h * 60)}
                </span>
              )}
            </For>
          </div>
          <For each={props.days}>
            {(day) => (
              <div class="week-col" classList={{ today: day === props.today }} data-day={day}>
                <For each={segments().get(day) ?? []}>
                  {(s) => (
                    <a
                      class="period-event week-event"
                      classList={{ "continues-before": s.continuesBefore, "continues-after": s.continuesAfter, short: s.end - s.start < 45 }}
                      style={style(s)}
                      href={`/notes/${s.event.id}`}
                      data-testid="period-event"
                      data-id={s.event.id}
                      title={s.continuesBefore ? s.event.title : `${clock(s.start)} ${s.event.title}`}
                    >
                      <span class="period-event-time">{s.continuesBefore ? "" : clock(s.start)}</span>
                      <span class="period-event-title">{s.event.title}</span>
                    </a>
                  )}
                </For>
                <Show when={day === props.today && now() >= hours().first * 60 && now() <= hours().last * 60}>
                  <div class="week-now" data-testid="period-now" style={{ top: `calc(var(--hour-height) * ${rows(now())})` }} />
                </Show>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}

/** A day's events, each a link to its note. */
function EventList(props: { events: CalendarEvent[]; day: string }) {
  return (
    <ul class="period-events">
      <For each={props.events}>
        {(e) => (
          <li>
            <a class="period-event" classList={{ "all-day": e.allDay }} href={`/notes/${e.id}`} data-testid="period-event" data-id={e.id} title={e.title}>
              <Show when={eventTime(e, props.day)}>{(t) => <span class="period-event-time">{t()}</span>}</Show>
              <span class="period-event-title">{e.title}</span>
            </a>
          </li>
        )}
      </For>
    </ul>
  );
}

/** A month's grid from Monday, each day with its events. */
function MonthGrid(props: { month: string; today: string; eventsOn: (day: string) => CalendarEvent[]; title: boolean }) {
  const weeks = createMemo(() => monthGrid(monthOf(props.month), 1));
  return (
    <div class="period-month">
      <Show when={props.title}>
        <h3 class="period-month-title">
          <a href={plannerHref("month", props.month, props.today)}>{periodTitle("month", props.month)}</a>
        </h3>
      </Show>
      <table class="period-month-grid">
        <thead>
          <tr>
            <For each={WEEKDAYS}>{(w) => <th scope="col">{w}</th>}</For>
          </tr>
        </thead>
        <tbody>
          <For each={weeks()}>
            {(week) => (
              <tr>
                <For each={week}>
                  {(cell) => (
                    <td classList={{ outside: !cell.inMonth, today: cell.day === props.today }} data-day={cell.inMonth ? cell.day : undefined}>
                      <Show when={cell.inMonth}>
                        <a class="period-day-link" href={plannerHref("day", cell.day, props.today)} aria-label={longDay(cell.day)} data-testid="period-day">
                          {cell.date}
                        </a>
                        <EventList events={props.eventsOn(cell.day)} day={cell.day} />
                      </Show>
                    </td>
                  )}
                </For>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}

/** Twelve small months with a count on each day that has events; a click on a day lists them. */
function YearGrid(props: { first: string; today: string; eventsOn: (day: string) => CalendarEvent[] }) {
  const [picked, setPicked] = createSignal<string | null>(null);
  const months = () => Array.from({ length: 12 }, (_, i) => shiftPeriod("month", props.first, i));
  return (
    <div class="period-year">
      <div class="period-year-months">
        <For each={months()}>
          {(month) => (
            <div class="period-mini">
              <h3 class="period-month-title">
                <a href={plannerHref("month", month, props.today)}>{MONTHS[local(month).getMonth()]}</a>
              </h3>
              <table class="period-mini-grid">
                <thead>
                  <tr>
                    <For each={WEEKDAYS}>{(w) => <th scope="col" abbr={w}>{w.slice(0, 1)}</th>}</For>
                  </tr>
                </thead>
                <tbody>
                  <For each={monthGrid(monthOf(month), 1)}>
                    {(week) => (
                      <tr>
                        <For each={week}>
                          {(cell) => (
                            <td classList={{ outside: !cell.inMonth, today: cell.day === props.today, picked: cell.day === picked() }}>
                              <Show when={cell.inMonth}>
                                <button
                                  type="button"
                                  data-day={cell.day}
                                  data-testid="period-year-day"
                                  aria-pressed={cell.day === picked()}
                                  aria-label={`${longDay(cell.day)}${props.eventsOn(cell.day).length ? `, ${props.eventsOn(cell.day).length} events` : ""}`}
                                  classList={{ busy: props.eventsOn(cell.day).length > 0 }}
                                  onClick={() => setPicked(cell.day === picked() ? null : cell.day)}
                                >
                                  {cell.date}
                                  <Show when={props.eventsOn(cell.day).length}>
                                    {(n) => (
                                      <span class="period-count" data-testid="period-count">
                                        {n()}
                                      </span>
                                    )}
                                  </Show>
                                </button>
                              </Show>
                            </td>
                          )}
                        </For>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          )}
        </For>
      </div>
      <Show when={picked()}>
        {(day) => (
          <div class="period-picked" data-testid="period-picked">
            <a class="period-day-link" href={plannerHref("day", day(), props.today)} data-testid="period-day">
              {longDay(day())}
            </a>
            <Show when={props.eventsOn(day()).length} fallback={<p class="muted">Nothing on this day.</p>}>
              <EventList events={props.eventsOn(day())} day={day()} />
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}

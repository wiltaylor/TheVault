// A `period_calendar` block in a planner note's template (docs/vault-format.md,
// "period_calendar"): the notes of its query, events by default, on the days of the note's week,
// month, quarter or year. Every day links to its daily note; every event opens on click. Links
// are plain in-app links, so a click opens them in this pane and a middle or Ctrl/Cmd+click in a
// background tab, as any link does.

import { For, Show, createMemo, createResource, createSignal, useContext } from "solid-js";
import { type Block, type Row, api } from "../api";
import { changes } from "../changes";
import { FieldRefs } from "../components/fieldRefs";
import { monthGrid, monthOf, todayDay } from "../daily/calendar";
import { types } from "../session";
import { calendarEvent, dateField, narrow, visibleWindow, type CalendarEvent } from "../views/narrow";
import { useThisNote } from "../views/thisNote";
import { MONTHS, DAY, longDay, periodEnd, periodStart, periodTitle, plannerHref, shiftDay, shiftPeriod } from "./period";
import { type CalendarPeriod, eventTime, eventsByDay, readPeriodCalendar, sortDay } from "./periodCalendar";
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
  const byDay = createMemo(() => eventsByDay(rows.error ? [] : (rows.latest ?? []), first(), last()));
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
        <div class="period-week">
          <For each={Array.from({ length: 7 }, (_, i) => shiftDay(first(), i))}>
            {(day, i) => (
              <div class="period-week-day" classList={{ today: day === today }} data-day={day}>
                <a class="period-day-link" href={plannerHref("day", day, today)} aria-label={longDay(day)} data-testid="period-day">
                  <span class="period-weekday">{WEEKDAYS[i()]}</span>
                  <span class="period-date">{local(day).getDate()}</span>
                </a>
                <EventList events={eventsOn(day)} day={day} />
              </div>
            )}
          </For>
        </div>
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

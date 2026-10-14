import { useNavigate, useParams } from "@solidjs/router";
import { Match, Switch, createResource, createSignal } from "solid-js";
import { ApiError, type Note, api } from "../api";
import { NoteView } from "./NoteView";
import { errorText } from "./TypeEditor";

/** `d` as `YYYY-MM-DD` in the browser's own time zone. */
export function isoDay(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** The day `days` after `day`. */
export function shiftDay(day: string, days: number): string {
  const [y, m, d] = day.split("-").map(Number);
  return isoDay(new Date(y!, m! - 1, d! + days));
}

const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

/** `day` written out the way the server titles a daily note, such as `Tuesday 13 October 2026`. */
export function longDay(day: string): string {
  const [y, m, d] = day.split("-").map(Number);
  const date = new Date(y!, m! - 1, d!);
  return `${WEEKDAYS[date.getDay()]} ${d} ${MONTHS[m! - 1]} ${y}`;
}

const DAY = /^\d{4}-\d{2}-\d{2}$/;

/**
 * The note for one day, of the built-in `dailynote` type. Today's note is created on first visit;
 * another day with no note offers to create one, so browsing the calendar leaves no empty notes.
 */
export function DailyNotes() {
  const params = useParams();
  const navigate = useNavigate();
  const today = () => isoDay(new Date());
  const day = () => (params.date && DAY.test(params.date) ? params.date : today());
  const [creating, setCreating] = createSignal(false);

  const [note, { refetch }] = createResource(day, async (d): Promise<Note | null> => {
    if (d === today()) return api.openDaily(d);
    try {
      return await api.daily(d);
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) return null;
      throw e;
    }
  });

  const go = (d: string) => navigate(d === today() ? "/daily" : `/daily/${d}`);

  async function create() {
    setCreating(true);
    try {
      await api.openDaily(day());
      await refetch();
    } finally {
      setCreating(false);
    }
  }

  return (
    <section class="daily-page">
      <nav class="daily-nav" aria-label="Day">
        <button type="button" aria-label="Previous day" onClick={() => go(shiftDay(day(), -1))}>
          ‹
        </button>
        <input
          type="date"
          aria-label="Day"
          value={day()}
          onChange={(e) => {
            const v = e.currentTarget.value;
            if (DAY.test(v)) go(v);
          }}
        />
        <button type="button" aria-label="Next day" onClick={() => go(shiftDay(day(), 1))}>
          ›
        </button>
        <button type="button" onClick={() => go(today())} disabled={day() === today()}>
          Today
        </button>
      </nav>
      <Switch>
        <Match when={note.error}>
          <p class="error" role="alert">
            {errorText(note.error)}
          </p>
        </Match>
        <Match when={note.loading && !note.latest}>
          <p class="muted">Loading…</p>
        </Match>
        <Match when={note()}>{(n) => <NoteView id={n().id} />}</Match>
        <Match when={note() === null}>
          <div class="daily-empty">
            <h1>{longDay(day())}</h1>
            <p class="muted">There is no note for this day.</p>
            <button type="button" class="primary" disabled={creating()} onClick={() => void create()}>
              Create note for this day
            </button>
          </div>
        </Match>
      </Switch>
    </section>
  );
}

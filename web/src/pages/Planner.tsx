// The planner: one note per day, ISO week, month, quarter and year, of the built-in period types.
// The current period's note is created on first visit; another period with no note offers to
// create one, so browsing leaves no empty notes behind.

import { Navigate, useNavigate, useParams } from "@solidjs/router";
import { For, Match, Switch, createEffect, createResource, createSignal } from "solid-js";
import { ApiError, type Note, api } from "../api";
import {
  DAY,
  PERIODS,
  PERIOD_LABELS,
  type Period,
  isPeriod,
  lastPeriod,
  periodStart,
  periodTitle,
  plannerHref,
  rememberPeriod,
  shiftPeriod,
  today,
} from "../planner/period";
import { NoteView } from "./NoteView";
import { errorText } from "./TypeEditor";
import "../planner/planner.css";

/** `/daily` and `/daily/<day>`, the old daily notes page, open the planner's day. */
export function DailyRedirect() {
  const params = useParams();
  return <Navigate href={params.date && DAY.test(params.date) ? `/planner/day/${params.date}` : "/planner/day"} />;
}

export function Planner() {
  const params = useParams();
  const navigate = useNavigate();
  const period = (): Period => (isPeriod(params.period) ? params.period : lastPeriod());
  const day = () => periodStart(period(), params.date && DAY.test(params.date) ? params.date : today());
  const current = () => day() === periodStart(period(), today());
  const label = () => PERIOD_LABELS[period()];
  const noun = () => label().toLowerCase();
  const [creating, setCreating] = createSignal(false);

  // The period shown is the one chosen last, the next time the planner opens without one.
  createEffect(() => {
    if (isPeriod(params.period)) rememberPeriod(params.period);
  });

  const [note, { refetch }] = createResource(
    () => ({ period: period(), day: day(), current: current() }),
    async (k): Promise<Note | null> => {
      if (k.current) return api.openPlanner(k.period, k.day);
      try {
        return await api.planner(k.period, k.day);
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) return null;
        throw e;
      }
    },
  );

  const go = (p: Period, d: string) => navigate(plannerHref(p, d, today()));

  async function create() {
    setCreating(true);
    try {
      await api.openPlanner(period(), day());
      await refetch();
    } finally {
      setCreating(false);
    }
  }

  return (
    <section class="daily-page planner-page">
      <div class="planner-bar">
        <div class="segmented" role="group" aria-label="Period">
          <For each={PERIODS}>
            {(p) => (
              <button
                type="button"
                aria-pressed={p === period()}
                onClick={() => {
                  rememberPeriod(p);
                  // The new period holds the day shown, or today when the current period shows.
                  go(p, current() ? today() : day());
                }}
              >
                {PERIOD_LABELS[p]}
              </button>
            )}
          </For>
        </div>
        <nav class="daily-nav" aria-label={label()}>
          <button type="button" aria-label={`Previous ${noun()}`} onClick={() => go(period(), shiftPeriod(period(), day(), -1))}>
            ‹
          </button>
          <input
            type="date"
            aria-label={label()}
            value={day()}
            onChange={(e) => {
              const v = e.currentTarget.value;
              if (DAY.test(v)) go(period(), v);
            }}
          />
          <button type="button" aria-label={`Next ${noun()}`} onClick={() => go(period(), shiftPeriod(period(), day(), 1))}>
            ›
          </button>
          <button type="button" onClick={() => go(period(), today())} disabled={current()}>
            Today
          </button>
        </nav>
      </div>
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
            <h1>{periodTitle(period(), day())}</h1>
            <p class="muted">There is no note for this {noun()}.</p>
            <button type="button" class="primary" disabled={creating()} onClick={() => void create()}>
              Create note for this {noun()}
            </button>
          </div>
        </Match>
      </Switch>
    </section>
  );
}

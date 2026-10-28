// The editor of an event series' `rule` (event-series E6): a builder for one plain RRULE line,
// with its summary, and a "Custom RRULE" disclosure showing the raw lines. A rule the builder
// cannot show opens in raw mode, with a note saying why.

import { For, Match, Show, Switch, createEffect, createMemo, createSignal } from "solid-js";
import type { Json } from "../api";
import { useControlBinding } from "../controls/binding";
import {
  FREQS,
  type Freq,
  type MonthlyOn,
  type RuleEnd,
  type RuleModel,
  WEEKDAYS,
  WEEKDAY_LONG,
  WEEKDAY_SHORT,
  type Weekday,
  defaultRule,
  ordinal,
  readRule,
  ruleLine,
  ruleLines,
  ruleSummary,
} from "./rrule";

const FREQ_TEXT: Record<Freq, string> = { daily: "Daily", weekly: "Weekly", monthly: "Monthly", yearly: "Yearly" };
const UNIT: Record<Freq, string> = { daily: "day(s)", weekly: "week(s)", monthly: "month(s)", yearly: "year(s)" };

/** The stored value for raw text: one line per line, nothing for none. */
function rawValue(text: string): Json | undefined {
  const lines = ruleLines(text);
  return lines.length ? lines : undefined;
}

export function RuleEditor(props: { value: Json | undefined; onChange: (v: Json | undefined) => void; label: string; id: string }) {
  const binding = useControlBinding();
  // Whether the series' first occurrence is a date, so UNTIL is written as a date too.
  const allDay = () => {
    const starts = binding?.type()?.fields.find((f) => f.name === "starts");
    const v = starts ? binding?.value(starts) : undefined;
    const start = v && typeof v === "object" && !Array.isArray(v) ? v.start : undefined;
    return typeof start === "string" && start.length === 10;
  };
  const read = createMemo(() => readRule(props.value));
  const empty = () => ruleLines(props.value).length === 0;
  // Raw mode: the rule cannot be built, or the user opened the raw lines and is typing in them.
  const [rawOpen, setRawOpen] = createSignal(false);
  const raw = () => read().reason !== null;
  const model = (): RuleModel => read().model ?? defaultRule();
  const [text, setText] = createSignal(ruleLines(props.value).join("\n"));
  let area: HTMLTextAreaElement | undefined;
  createEffect(() => {
    const t = ruleLines(props.value).join("\n");
    if (document.activeElement !== area) setText(t);
  });

  const write = (fn: (m: RuleModel) => void) => {
    const m = structuredClone(model());
    fn(m);
    props.onChange([ruleLine(m, allDay())]);
  };
  const setFreq = (f: Freq) => write((m) => (m.freq = f));
  const toggleDay = (d: Weekday) =>
    write((m) => (m.weekdays = m.weekdays.includes(d) ? m.weekdays.filter((x) => x !== d) : [...m.weekdays, d]));
  const setMonthly = (on: MonthlyOn) => write((m) => (m.monthly = on));
  const setEnd = (end: RuleEnd) => write((m) => (m.end = end));
  const id = (part: string) => `${props.id}-${part}`;
  const num = (s: string, min: number) => Math.max(min, Math.floor(Number(s) || min));

  return (
    <div class="rule-editor" role="group" aria-label={props.label} id={props.id}>
      <Show
        when={!raw()}
        fallback={
          <p class="rule-raw-note muted" data-testid="rule-raw-note">
            {read().reason} Edit it as RRULE lines below.
          </p>
        }
      >
        <div class="rule-row">
          <label for={id("freq")}>Repeats</label>
          <select id={id("freq")} onChange={(e) => setFreq(e.currentTarget.value as Freq)}>
            <Show when={empty()}>
              <option value="" selected>
                Choose…
              </option>
            </Show>
            <For each={FREQS}>
              {(f) => (
                <option value={f} selected={!empty() && model().freq === f}>
                  {FREQ_TEXT[f]}
                </option>
              )}
            </For>
          </select>
          <Show when={!empty()}>
            <label for={id("interval")}>every</label>
            <input
              id={id("interval")}
              type="number"
              min="1"
              step="1"
              class="rule-interval"
              value={model().interval}
              onChange={(e) => write((m) => (m.interval = num(e.currentTarget.value, 1)))}
            />
            <span>{UNIT[model().freq]}</span>
          </Show>
        </div>
        <Show when={!empty()}>
          <Show when={model().freq === "weekly"}>
            <div class="rule-row rule-weekdays" role="group" aria-label="On">
              <For each={WEEKDAYS}>
                {(d) => (
                  <button
                    type="button"
                    class="rule-day"
                    aria-pressed={model().weekdays.includes(d)}
                    aria-label={WEEKDAY_LONG[d]}
                    onClick={() => toggleDay(d)}
                  >
                    {WEEKDAY_SHORT[d]}
                  </button>
                )}
              </For>
            </div>
          </Show>
          <Show when={model().freq === "monthly"}>
            <div class="rule-row">
              <label for={id("monthly")}>On</label>
              <select
                id={id("monthly")}
                onChange={(e) => {
                  const by = e.currentTarget.value;
                  setMonthly(by === "day" ? { by: "day", day: 1 } : by === "weekday" ? { by: "weekday", nth: 1, weekday: "MO" } : { by: "start" });
                }}
              >
                <option value="start" selected={model().monthly.by === "start"}>
                  the same day as the first
                </option>
                <option value="day" selected={model().monthly.by === "day"}>
                  day of the month
                </option>
                <option value="weekday" selected={model().monthly.by === "weekday"}>
                  a weekday of the month
                </option>
              </select>
              <Switch>
                <Match when={model().monthly.by === "day" && (model().monthly as { day: number })}>
                  {(on) => (
                    <select aria-label="Day of the month" onChange={(e) => setMonthly({ by: "day", day: Number(e.currentTarget.value) })}>
                      <For each={[...Array.from({ length: 31 }, (_, i) => i + 1), -1]}>
                        {(n) => (
                          <option value={n} selected={on().day === n}>
                            {n === -1 ? "last" : n}
                          </option>
                        )}
                      </For>
                    </select>
                  )}
                </Match>
                <Match when={model().monthly.by === "weekday" && (model().monthly as { nth: number; weekday: Weekday })}>
                  {(on) => (
                    <>
                      <select aria-label="Which" onChange={(e) => setMonthly({ by: "weekday", nth: Number(e.currentTarget.value), weekday: on().weekday })}>
                        <For each={[1, 2, 3, 4, -1]}>
                          {(n) => (
                            <option value={n} selected={on().nth === n}>
                              {ordinal(n)}
                            </option>
                          )}
                        </For>
                      </select>
                      <select aria-label="Weekday" onChange={(e) => setMonthly({ by: "weekday", nth: on().nth, weekday: e.currentTarget.value as Weekday })}>
                        <For each={WEEKDAYS}>
                          {(d) => (
                            <option value={d} selected={on().weekday === d}>
                              {WEEKDAY_LONG[d]}
                            </option>
                          )}
                        </For>
                      </select>
                    </>
                  )}
                </Match>
              </Switch>
            </div>
          </Show>
          <div class="rule-row">
            <label for={id("end")}>Ends</label>
            <select
              id={id("end")}
              onChange={(e) => {
                const k = e.currentTarget.value;
                const today = new Date();
                const inAYear = `${today.getFullYear() + 1}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
                setEnd(k === "until" ? { kind: "until", date: inAYear } : k === "count" ? { kind: "count", count: 10 } : { kind: "never" });
              }}
            >
              <option value="never" selected={model().end.kind === "never"}>
                Never
              </option>
              <option value="until" selected={model().end.kind === "until"}>
                On a date
              </option>
              <option value="count" selected={model().end.kind === "count"}>
                After a number of times
              </option>
            </select>
            <Switch>
              <Match when={model().end.kind === "until" && (model().end as { date: string })}>
                {(end) => (
                  <input
                    type="date"
                    aria-label="Until"
                    value={end().date}
                    onChange={(e) => e.currentTarget.value && setEnd({ kind: "until", date: e.currentTarget.value })}
                  />
                )}
              </Match>
              <Match when={model().end.kind === "count" && (model().end as { count: number })}>
                {(end) => (
                  <>
                    <input
                      type="number"
                      min="1"
                      step="1"
                      class="rule-interval"
                      aria-label="Times"
                      value={end().count}
                      onChange={(e) => setEnd({ kind: "count", count: num(e.currentTarget.value, 1) })}
                    />
                    <span>times</span>
                  </>
                )}
              </Match>
            </Switch>
          </div>
        </Show>
      </Show>
      <Show when={!empty()}>
        <p class="rule-summary" data-testid="rule-summary">
          {ruleSummary(props.value)}
        </p>
      </Show>
      <details class="rule-raw" open={raw() || rawOpen()} onToggle={(e) => setRawOpen(e.currentTarget.open)}>
        <summary>Custom RRULE</summary>
        <textarea
          ref={area}
          aria-label="RRULE lines"
          rows={Math.max(2, text().split("\n").length)}
          spellcheck={false}
          value={text()}
          placeholder="RRULE:FREQ=WEEKLY;BYDAY=MO"
          onInput={(e) => {
            setText(e.currentTarget.value);
            props.onChange(rawValue(e.currentTarget.value));
          }}
        />
      </details>
    </div>
  );
}

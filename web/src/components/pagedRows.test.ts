import { createEffect, createRoot, createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import type { Page, Row } from "../api";
import { createPagedRows } from "./pagedRows";

const row = (id: string, stage: string): Row => ({ id, type: "story", title: id, fields: { stage } });

/** A paged result over `all()`, `size` rows a page, recording every cursor asked for. */
function source(all: () => Row[]) {
  const asked: (string | null)[] = [];
  const fetchPage = async (cursor: string | null, size: number): Promise<Page> => {
    asked.push(cursor);
    const from = cursor ? Number(cursor) : 0;
    const rows = all().slice(from, from + size);
    const end = from + rows.length;
    return { rows, next: end < all().length ? String(end) : null, total: all().length };
  };
  return { asked, fetchPage };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("createPagedRows", () => {
  it("loads every page it holds again when refresh changes, and swaps the rows in without emptying", async () => {
    let all = [row("a", "Todo"), row("b", "Todo"), row("c", "Todo")];
    const { asked, fetchPage } = source(() => all);
    const [tick, setTick] = createSignal(0);
    const data = createRoot(() => createPagedRows(fetchPage, () => "q", 2, tick));
    await settle();
    await data.loadUntil(2);
    expect(data.rows().map((r) => r.id)).toEqual(["a", "b", "c"]);
    expect(asked).toEqual([null, "2"]);

    all = [row("a", "Doing"), row("b", "Todo"), row("c", "Todo")];
    // Every row count the list passes through, to see whether it ever empties.
    const seen: number[] = [];
    createRoot(() => createEffect(() => seen.push(data.rows().length)));
    setTick(1);
    await settle();
    await settle();
    expect(asked).toEqual([null, "2", null, "2"]);
    expect(data.rows().map((r) => r.fields.stage)).toEqual(["Doing", "Todo", "Todo"]);
    expect(data.total()).toBe(3);
    expect(seen.every((n) => n > 0)).toBe(true);
  });

  it("does not load anything when refresh is first read", async () => {
    const { asked, fetchPage } = source(() => [row("a", "Todo")]);
    const [tick] = createSignal(0);
    createRoot(() => createPagedRows(fetchPage, () => "q", 2, tick));
    await settle();
    expect(asked).toEqual([null]);
  });
});

import { afterEach, expect, it, vi } from "vitest";
import { api } from "../api";
import { countInbox } from "./load";

afterEach(() => vi.restoreAllMocks());

it("uses four filtered totals without paging rows or held notes, regardless of inbox size", async () => {
  const held = vi.spyOn(api, "heldAgentNotes");
  const query = vi.spyOn(api, "query").mockImplementation(async () => {
    return { rows: [], next: "more", total: 10000 };
  });
  expect(await countInbox()).toBe(40000);
  expect(held).not.toHaveBeenCalled();
  expect(query).toHaveBeenCalledTimes(4);
  for (const call of query.mock.calls) {
    expect(call.slice(1)).toEqual([null, 1, undefined, undefined, true]);
  }
});

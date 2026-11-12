import { afterEach, expect, it, vi } from "vitest";
import { api } from "../api";
import { countInbox } from "./load";

afterEach(() => vi.restoreAllMocks());

it("counts allowed inbox rows, not held rows, including subsequent pages", async () => {
  vi.spyOn(api, "heldAgentNotes").mockResolvedValue({ notes: [{ id: "held" } as never, { id: "outside" } as never], total: 2 });
  vi.spyOn(api, "query").mockImplementation(async (query, cursor) => {
    if (!query.startsWith("from email")) return { rows: [], next: null, total: 0 };
    return cursor
      ? { rows: [{ id: "allowed" } as never], next: null, total: 2 }
      : { rows: [{ id: "held" } as never], next: "next", total: 2 };
  });
  expect(await countInbox()).toBe(1);
});

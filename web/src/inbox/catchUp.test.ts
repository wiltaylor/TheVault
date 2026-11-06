import { describe, expect, it } from "vitest";
import { startOf, summary, today } from "./CatchUpDialog";

describe("the inbox catch-up", () => {
  it("starts a date at local midnight", () => {
    const at = startOf("2026-11-01");
    expect(at).not.toBeNull();
    const d = new Date(at ?? "");
    expect([d.getFullYear(), d.getMonth(), d.getDate(), d.getHours(), d.getMinutes()]).toEqual([2026, 10, 1, 0, 0]);
    expect(startOf("1 Nov")).toBeNull();
  });

  it("gives today's local date", () => {
    expect(today(new Date(2026, 10, 6, 23, 30))).toBe("2026-11-06");
  });

  it("says what a dry run found, or that nothing waits", () => {
    const base = { dry_run: true, changed: 0, chunks: 0 };
    expect(summary({ ...base, matched: { email: 1971, message: 1, call: 0 }, total: 1972 }, "2026-11-01")).toBe(
      "1972 items sent or made before 2026-11-01 will be marked processed: 1971 emails, 1 message.",
    );
    expect(summary({ ...base, matched: { email: 0, message: 0, call: 0 }, total: 0 }, "2026-11-01")).toBe(
      "Nothing sent or made before 2026-11-01 waits to be processed.",
    );
  });
});

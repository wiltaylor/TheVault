import { describe, expect, it } from "vitest";
import { generalActions } from "./palette/actions";
import { type ImportKindReport, describeKind, importPayload, missingPermissions } from "./deviceImport";

const NOW = new Date("2026-10-16T12:00:00Z");

function kind(change: Partial<ImportKindReport>): ImportKindReport {
  return {
    kind: "sms",
    found: 0,
    imported: 0,
    notes: 0,
    existing: 0,
    skipped: 0,
    failed: 0,
    missing_permission: null,
    error: null,
    done: true,
    ...change,
  };
}

describe("importPayload", () => {
  it("turns the choices into kinds, SMS and MMS together", () => {
    const p = importPayload({ choices: ["contacts", "messages"], preset: "all", from: "", to: "" }, NOW);
    expect(p).toEqual({ kinds: ["sms", "mms", "contacts"], since: null, until: null });
  });

  it("counts presets back from now, open-ended", () => {
    expect(importPayload({ choices: ["calls"], preset: "7d", from: "", to: "" }, NOW)).toEqual({
      kinds: ["calls"],
      since: "2026-10-09T12:00:00.000Z",
      until: null,
    });
    const year = importPayload({ choices: ["calls"], preset: "1y", from: "", to: "" }, NOW);
    expect(typeof year === "object" && year.since).toBe("2025-10-16T12:00:00.000Z");
  });

  it("runs a custom range from the start of its first day to the end of its last, in local time", () => {
    const p = importPayload({ choices: ["messages"], preset: "custom", from: "2026-10-01", to: "2026-10-02" }, NOW);
    if (typeof p === "string") throw new Error(p);
    expect(p.since).toBe(new Date(2026, 9, 1).toISOString());
    expect(new Date(p.until!).getTime()).toBe(new Date(2026, 9, 3).getTime() - 1);
  });

  it("refuses nothing chosen and a backwards range", () => {
    expect(importPayload({ choices: [], preset: "all", from: "", to: "" }, NOW)).toBe("Choose what to import.");
    expect(importPayload({ choices: ["calls"], preset: "custom", from: "2026-10-05", to: "2026-10-01" }, NOW)).toBe(
      "The start date is after the end date.",
    );
  });
});

describe("reports", () => {
  it("say what was found, what was new and what was already there", () => {
    expect(describeKind(kind({ found: 12, notes: 3, existing: 9 }))).toBe("SMS: 12 found, 3 new notes, 9 already in the vault");
    expect(describeKind(kind({ kind: "calls", found: 1, notes: 1, done: false }))).toBe("Calls: 1 found, 1 new note…");
  });

  it("name a missing permission once", () => {
    const missing = kind({ kind: "mms", missing_permission: "android.permission.READ_SMS" });
    expect(describeKind(missing)).toBe("MMS: not read. Android has not given TheVault READ_SMS.");
    const report = { kinds: [missing, { ...missing, kind: "sms" as const }], since: null, until: null, done: true };
    expect(missingPermissions(report)).toEqual(["android.permission.READ_SMS"]);
  });
});

describe("the palette", () => {
  it("offers the import only inside the Android app", () => {
    const ids = (androidApp: boolean) => generalActions({ phone: true, dark: false, androidApp }).map((a) => a.id);
    expect(ids(false)).not.toContain("device:import");
    const action = generalActions({ phone: true, dark: false, androidApp: true }).find((a) => a.id === "device:import");
    expect(action?.label).toBe("Import messages and calls");
    expect(action?.href).toBe("/settings/devices#this-device");
  });
});

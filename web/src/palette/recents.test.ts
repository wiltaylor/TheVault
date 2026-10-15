import { beforeEach, describe, expect, it } from "vitest";
import { MAX_RECENTS, RECENTS_KEY, forgetRecent, noteAt, recents, recordRecent, reloadRecents, updateRecent } from "./recents";

const ID = (n: number) => `01JB8Z4K2M${String(n).padStart(16, "0")}`;

beforeEach(() => {
  localStorage.clear();
  reloadRecents();
});

describe("recents", () => {
  it("puts the latest note first and keeps each note once", () => {
    recordRecent(ID(1), "One", "task", 1);
    recordRecent(ID(2), "Two", "task", 2);
    recordRecent(ID(1), undefined, undefined, 3);
    expect(recents().map((r) => [r.id, r.title, r.at])).toEqual([
      [ID(1), "One", 3],
      [ID(2), "Two", 2],
    ]);
  });

  it("keeps at most MAX_RECENTS", () => {
    for (let i = 0; i < MAX_RECENTS + 5; i++) recordRecent(ID(i), `N${i}`, "task", i);
    expect(recents()).toHaveLength(MAX_RECENTS);
    expect(recents()[0]!.id).toBe(ID(MAX_RECENTS + 4));
  });

  it("is kept in localStorage and read back", () => {
    recordRecent(ID(7), "Seven", "note", 7);
    expect(JSON.parse(localStorage.getItem(RECENTS_KEY)!)).toEqual([{ id: ID(7), title: "Seven", type: "note", at: 7 }]);
    localStorage.setItem(RECENTS_KEY, JSON.stringify([{ id: ID(8), title: "Eight", type: "note", at: 8 }, { bad: true }]));
    reloadRecents();
    expect(recents().map((r) => r.id)).toEqual([ID(8)]);
    localStorage.setItem(RECENTS_KEY, "not json");
    reloadRecents();
    expect(recents()).toEqual([]);
  });

  it("follows a rename in place, and forgets a note", () => {
    recordRecent(ID(1), "One", "task", 1);
    recordRecent(ID(2), "Two", "task", 2);
    updateRecent(ID(1), "Uno");
    expect(recents().map((r) => r.title)).toEqual(["Two", "Uno"]);
    forgetRecent(ID(2));
    expect(recents().map((r) => r.id)).toEqual([ID(1)]);
  });
});

describe("noteAt", () => {
  it("reads the note a location shows", () => {
    expect(noteAt(`/notes/${ID(1)}`)).toBe(ID(1));
    expect(noteAt(`/notes/${ID(1)}?tab=2`)).toBe(ID(1));
    expect(noteAt("/notes/new?type=task")).toBeNull();
    expect(noteAt("/search?q=x")).toBeNull();
  });
});

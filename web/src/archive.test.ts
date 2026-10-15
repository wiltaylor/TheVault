import { describe, expect, it, vi } from "vitest";
import { isArchived } from "./api";
import { archiveQuery } from "./archive";
import { noteActions } from "./palette/actions";
import { withoutArchived } from "./palette/model";

describe("archiveQuery", () => {
  it("reads archived notes of every type or one, newest change first", () => {
    expect(archiveQuery("", null)).toBe("from * where archived sort updated desc, id desc select updated");
    expect(archiveQuery("  ", "task")).toBe("from task where archived sort updated desc, id desc select updated");
  });

  it("narrows by full text, keeping one trailing space as the Search page does", () => {
    expect(archiveQuery("old plan", null)).toBe('from * where archived search "old plan" sort updated desc, id desc select updated');
    expect(archiveQuery("old ", "task")).toBe('from task where archived search "old " sort updated desc, id desc select updated');
    expect(archiveQuery('say "hi"', null)).toBe('from * where archived search "say \\"hi\\"" sort updated desc, id desc select updated');
  });
});

describe("isArchived", () => {
  it("is true only for archived = true", () => {
    expect(isArchived({ archived: true })).toBe(true);
    expect(isArchived({ archived: false })).toBe(false);
    expect(isArchived({ archived: "true" })).toBe(false);
    expect(isArchived({})).toBe(false);
    expect(isArchived(undefined)).toBe(false);
  });
});

describe("the command palette and archived notes", () => {
  it("leaves out recent notes known to be archived, and keeps those not yet loaded", () => {
    const state: Record<string, boolean | undefined> = { a: true, b: false };
    const recents = [{ id: "a" }, { id: "b" }, { id: "c" }];
    expect(withoutArchived(recents, (id) => state[id]).map((r) => r.id)).toEqual(["b", "c"]);
  });

  it("offers Archive or Unarchive for the note the active pane shows", () => {
    expect(noteActions(null)).toEqual([]);
    const toggle = vi.fn(async () => {});
    const [archive] = noteActions({ owner: "p", id: "n", archived: false, toggle });
    expect(archive?.label).toBe("Archive this note");
    expect(archive?.general).toBe(true);
    archive?.run?.();
    expect(toggle).toHaveBeenCalledOnce();
    const [unarchive] = noteActions({ owner: "p", id: "n", archived: true, toggle });
    expect(unarchive?.label).toBe("Unarchive this note");
  });
});

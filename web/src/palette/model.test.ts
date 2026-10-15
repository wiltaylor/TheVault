import { describe, expect, it } from "vitest";
import { type ActionLike, fuzzy, notesQuery, parseInput, rankActions, rankNotes, resolveType, segments, wordPositions } from "./model";
import { isPaletteKey } from "./store";

describe("fuzzy", () => {
  it("matches characters in order, ignoring case and spaces", () => {
    expect(fuzzy("nt", "New Task")?.positions).toEqual([0, 4]);
    expect(fuzzy("new task", "New Task")?.positions).toEqual([0, 1, 2, 4, 5, 6, 7]);
    expect(fuzzy("tn", "New Task")).toBeNull();
    expect(fuzzy("x", "New Task")).toBeNull();
  });

  it("prefers starts of words and runs over scattered characters", () => {
    // "gti" picks Go To Inbox's word starts.
    expect(fuzzy("gti", "Go to Inbox")?.positions).toEqual([0, 3, 6]);
    const words = fuzzy("sr", "Split right")!.score;
    const scattered = fuzzy("sr", "Superstar")!.score;
    expect(words).toBeGreaterThan(scattered);
  });

  it("scores the whole text above a prefix above a substring", () => {
    const exact = fuzzy("inbox", "Inbox")!.score;
    const prefix = fuzzy("inbox", "Inbox zero")!.score;
    const inside = fuzzy("inbox", "Go to Inbox")!.score;
    expect(exact).toBeGreaterThan(prefix);
    expect(prefix).toBeGreaterThan(inside);
  });

  it("is quick over thousands of labels", () => {
    const labels = Array.from({ length: 5000 }, (_, i) => `Go to Type number ${i} with a longer label`);
    const t = performance.now();
    for (const l of labels) fuzzy("go typ 42", l);
    expect(performance.now() - t).toBeLessThan(500);
  });
});

describe("segments and wordPositions", () => {
  it("cuts text into highlighted runs", () => {
    expect(segments("New Task", [0, 4, 5])).toEqual([
      { text: "N", hit: true },
      { text: "ew ", hit: false },
      { text: "Ta", hit: true },
      { text: "sk", hit: false },
    ]);
  });

  it("highlights each word where it starts a word of the title", () => {
    expect(wordPositions("plan", "Explain the planning")).toEqual([12, 13, 14, 15]);
    expect(wordPositions("pla", "Explain")).toEqual([2, 3, 4]);
    expect(wordPositions("", "Anything")).toEqual([]);
  });
});

describe("parseInput", () => {
  it("reads > as actions only", () => {
    expect(parseInput(">new task")).toEqual({ mode: "actions", text: "new task", typeWord: "" });
    expect(parseInput("  > go")).toEqual({ mode: "actions", text: "go", typeWord: "" });
  });

  it("reads # and type: as a type filter, keeping the trailing space", () => {
    expect(parseInput("#task plan ")).toEqual({ mode: "type", typeWord: "task", text: "plan " });
    expect(parseInput("type:project")).toEqual({ mode: "type", typeWord: "project", text: "" });
    expect(parseInput("#")).toEqual({ mode: "type", typeWord: "", text: "" });
  });

  it("reads anything else as notes and actions", () => {
    expect(parseInput("weekly plan")).toEqual({ mode: "all", text: "weekly plan", typeWord: "" });
    expect(parseInput("C# notes")).toEqual({ mode: "all", text: "C# notes", typeWord: "" });
  });
});

describe("resolveType", () => {
  const types = [
    { name: "task", label: "Task", plural: "Tasks" },
    { name: "project", label: "Project", plural: "Projects" },
    { name: "person", label: "Person", plural: "People" },
  ];
  it("finds a type by name, label or plural, else by the best fuzzy match", () => {
    expect(resolveType(types, "Tasks")?.name).toBe("task");
    expect(resolveType(types, "people")?.name).toBe("person");
    expect(resolveType(types, "proj")?.name).toBe("project");
    expect(resolveType(types, "zzz")).toBeNull();
    expect(resolveType(types, "")).toBeNull();
  });
});

describe("notesQuery", () => {
  it("searches every type, the last word as a prefix until a space follows", () => {
    expect(notesQuery("weekly pl", null)).toBe('from * search "weekly pl" select updated');
    expect(notesQuery("weekly ", null)).toBe('from * search "weekly " select updated');
  });

  it("lists a type's latest notes with nothing typed, and nothing at all without a type", () => {
    expect(notesQuery("", "task")).toBe("from task sort updated desc select updated");
    expect(notesQuery("  ", null)).toBeNull();
    expect(notesQuery('say "hi"', "task")).toBe('from task search "say \\"hi\\"" select updated');
  });
});

describe("rankNotes", () => {
  const n = (id: string, title: string) => ({ id, title, type: "task" });
  it("puts exact titles, then prefixes, then word prefixes, then recent notes, then the rest", () => {
    const found = [n("a", "A plan for later"), n("b", "Planning"), n("c", "Plan"), n("d", "Some other"), n("e", "Weekly planning")];
    const recents = [n("d", "Some other")];
    expect(rankNotes("plan", found, recents, 10).map((x) => x.id)).toEqual(["c", "b", "a", "e", "d"]);
  });

  it("adds recent notes whose titles hold the text before the search answers", () => {
    const ranked = rankNotes("week", [], [n("r1", "Weekly review"), n("r2", "Groceries")], 10);
    expect(ranked.map((x) => x.id)).toEqual(["r1"]);
    expect(ranked[0]!.recent).toBe(true);
    expect(ranked[0]!.positions).toEqual([0, 1, 2, 3]);
  });

  it("lists each note once, and no more than the limit", () => {
    const found = [n("a", "Alpha"), n("a", "Alpha"), n("b", "Alpha two"), n("c", "Alpha three")];
    expect(rankNotes("alpha", found, [n("a", "Alpha")], 2).map((x) => x.id)).toEqual(["a", "b"]);
  });

  it("keeps the server's order with nothing typed", () => {
    expect(rankNotes("", [n("x", "X"), n("y", "Y")], [], 5).map((x) => x.id)).toEqual(["x", "y"]);
  });
});

describe("rankActions", () => {
  const actions: (ActionLike & { id: string })[] = [
    { id: "capture", label: "Capture", keywords: ["Fleeting note"], general: true },
    { id: "inbox", label: "Go to Inbox", general: true },
    { id: "split", label: "Split right", general: true },
    { id: "new:task", label: "New Task", keywords: ["New Tasks"] },
    { id: "list:task", label: "Go to Tasks", keywords: ["Tasks", "task"] },
    { id: "new:note", label: "New Note", keywords: ["New Notes"] },
  ];

  it("lists only the general actions with nothing typed, or all of them when asked", () => {
    expect(rankActions(actions, "", 10).map((r) => r.action.id)).toEqual(["capture", "inbox", "split"]);
    expect(rankActions(actions, " ", 10, true).map((r) => r.action.id)).toEqual(["capture", "inbox", "split", "new:task", "list:task", "new:note"]);
  });

  it("ranks by fuzzy score, with word-start matches strong", () => {
    const r = rankActions(actions, "new task", 10);
    expect(r[0]!.action.id).toBe("new:task");
    expect(r[0]!.strong).toBe(true);
    expect(rankActions(actions, "inbox", 10)[0]!.action.id).toBe("inbox");
    expect(rankActions(actions, "tasks", 10)[0]!.action.id).toBe("list:task");
  });

  it("matches keywords, without highlighting the label", () => {
    const r = rankActions(actions, "fleeting", 10);
    expect(r[0]!.action.id).toBe("capture");
    expect(r[0]!.positions).toEqual([]);
  });

  it("drops actions that do not match, or match only scattered letters", () => {
    expect(rankActions(actions, "zzz", 10)).toEqual([]);
    expect(rankActions([{ label: "New Crew" }, { label: "Weekly" }], "we", 10).map((r) => r.action.label)).toEqual(["Weekly"]);
  });

  it("stays fast with hundreds of types", () => {
    const many = Array.from({ length: 800 }, (_, i) => ({ label: `New Type${i}`, keywords: [`Types${i}`, `type${i}`] }));
    const t = performance.now();
    const r = rankActions(many, "new type42", 10);
    expect(performance.now() - t).toBeLessThan(300);
    expect(r[0]!.action.label).toBe("New Type42");
  });
});

describe("isPaletteKey", () => {
  const key = (init: KeyboardEventInit, target?: EventTarget) => {
    const e = new KeyboardEvent("keydown", { code: "KeyK", ...init });
    if (target) Object.defineProperty(e, "target", { value: target });
    return e;
  };
  it("is Ctrl+K or Cmd+K, without Shift or Alt", () => {
    expect(isPaletteKey(key({ ctrlKey: true }), false)).toBe(true);
    expect(isPaletteKey(key({ metaKey: true }), true)).toBe(true);
    expect(isPaletteKey(key({ ctrlKey: true, shiftKey: true }), false)).toBe(false);
    expect(isPaletteKey(key({ ctrlKey: true, altKey: true }), false)).toBe(false);
    expect(isPaletteKey(key({}), false)).toBe(false);
  });

  it("leaves a Mac's Ctrl+K to the text it is typed in", () => {
    const input = document.createElement("input");
    expect(isPaletteKey(key({ ctrlKey: true }, input), true)).toBe(false);
    expect(isPaletteKey(key({ ctrlKey: true }, input), false)).toBe(true);
    expect(isPaletteKey(key({ ctrlKey: true }, document.body), true)).toBe(true);
  });
});

describe("rankActions keywords", () => {
  it("lets a label beat an action that only has the word as a keyword", () => {
    const actions = [
      { label: "Capture", keywords: ["Inbox"], general: true },
      { label: "Go to Inbox", general: true },
    ];
    expect(rankActions(actions, "inbox", 5)[0]!.action.label).toBe("Go to Inbox");
  });
});

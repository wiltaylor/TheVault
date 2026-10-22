import { beforeEach, describe, expect, it } from "vitest";
import type { FieldDef, TypeDef } from "./api";
import {
  TYPE_PREFS_KEY,
  TYPE_VIEWS_KEY,
  flipSort,
  listColumns,
  listPageSettings,
  loadTypePrefs,
  loadTypeView,
  nextSort,
  noteCount,
  pageStatus,
  parseSort,
  saveTypePrefs,
  saveTypeView,
  typeQuery,
} from "./typeList";

const field = (name: string, kind = "string", extra: Partial<FieldDef> = {}): FieldDef => ({
  name,
  label: name[0]!.toUpperCase() + name.slice(1),
  description: null,
  kind,
  required: false,
  computed: false,
  multiple: false,
  compound: false,
  ...extra,
});

const type = (fields: FieldDef[], card?: string[]): TypeDef => ({
  name: "book",
  label: "Book",
  plural: "Books",
  description: null,
  icon: null,
  colour: null,
  fields,
  templates: [
    { name: "default", label: null, tabs: [{ title: "Main", items: fields.map((f) => ({ slot: f.name, label: null })) }] },
    ...(card ? [{ name: "card", label: null, tabs: [{ title: "Card", items: card.map((s) => ({ slot: s, label: null })) }] }] : []),
  ],
});


describe("type list", () => {
  beforeEach(() => localStorage.clear());

  it("remembers the view per type, Cards by default", () => {
    expect(loadTypeView("book")).toBe("cards");
    saveTypeView("book", "list");
    expect(loadTypeView("book")).toBe("list");
    expect(loadTypeView("film")).toBe("cards");
    saveTypeView("book", "cards");
    expect(loadTypeView("book")).toBe("cards");
    localStorage.setItem(TYPE_VIEWS_KEY, "not json");
    expect(loadTypeView("book")).toBe("cards");
  });

  it("takes the List columns from the card template, else the first scalar fields", () => {
    const fields = [
      field("author"),
      field("body", "content", { compound: true }),
      field("pages", "int"),
      field("rating", "int"),
      field("shelf"),
    ];
    expect(listColumns(type(fields, ["title", "rating", "body", "author"])).map((f) => f.name)).toEqual(["rating", "author"]);
    expect(listColumns(type(fields)).map((f) => f.name)).toEqual(["author", "pages", "rating"]);
    expect(listColumns(type(fields, ["title"])).map((f) => f.name)).toEqual(["author", "pages", "rating"]);
  });

  it("queries with the sort on the server, narrowed by search text", () => {
    const byTitle = { key: "title", dir: "asc" } as const;
    const bySent = { key: "sent", dir: "desc" } as const;
    expect(typeQuery("book", "  ", false, byTitle)).toBe("from book sort title asc");
    expect(typeQuery("book", "dune", false, byTitle)).toBe('from book search "dune" sort title asc');
    expect(typeQuery("book", "dune ", false, bySent)).toBe('from book search "dune " sort sent desc');
    expect(typeQuery("book", 'say "hi"', false, byTitle)).toBe('from book search "say \\"hi\\"" sort title asc');
    expect(typeQuery("book", "", true, bySent)).toBe("from book include archived sort sent desc");
    expect(typeQuery("book", "dune", true, byTitle)).toBe('from book include archived search "dune" sort title asc');
  });

  it("reads the type's list settings, with the defaults for what it leaves out", () => {
    expect(listPageSettings(type([]))).toEqual({ view: "cards", sort: { key: "updated", dir: "desc" }, pageSize: 50 });
    expect(listPageSettings({ ...type([]), list_page: { view: "details", sort: "sent desc", page_size: 25 } })).toEqual({
      view: "details",
      sort: { key: "sent", dir: "desc" },
      pageSize: 25,
    });
    expect(listPageSettings({ ...type([]), list_page: { view: "table", sort: "a b c", page_size: 5000 } })).toEqual({
      view: "cards",
      sort: { key: "updated", dir: "desc" },
      pageSize: 50,
    });
    expect(parseSort("title")).toEqual({ key: "title", dir: "asc" });
    expect(parseSort("")).toBeUndefined();
  });

  it("remembers the user's view and sort per type, and forgets them on reset", () => {
    const book = type([]);
    expect(loadTypePrefs(book)).toEqual({});
    saveTypePrefs("book", { view: "details", sort: { key: "pages", dir: "desc" } });
    expect(loadTypePrefs(book)).toEqual({ view: "details", sort: { key: "pages", dir: "desc" } });
    expect(loadTypePrefs({ ...book, name: "film" })).toEqual({});
    saveTypePrefs("book", {});
    expect(loadTypePrefs(book)).toEqual({});
    expect(localStorage.getItem(TYPE_PREFS_KEY)).toBe("{}");
  });

  it("keeps an older List choice only for a type without list settings", () => {
    saveTypeView("book", "list");
    expect(loadTypePrefs(type([]))).toEqual({ view: "list" });
    expect(loadTypePrefs({ ...type([]), list_page: { view: "details" } })).toEqual({});
    saveTypePrefs("book", { view: "cards" });
    expect(loadTypeView("book")).toBe("cards");
  });

  it("says how many notes, or which of them a page shows", () => {
    expect(pageStatus(0, 50, 3, 3, false)).toBe("3 notes");
    expect(pageStatus(0, 50, 1, 1, true)).toBe("1 match");
    expect(pageStatus(0, 50, 0, 0, false)).toBe("0 notes");
    expect(pageStatus(0, 50, 50, 120, false)).toBe("1–50 of 120");
    expect(pageStatus(1, 50, 50, 120, false)).toBe("51–100 of 120");
    expect(pageStatus(2, 50, 20, 120, false)).toBe("101–120 of 120");
    expect(pageStatus(0, 50, 50, 1234, true)).toBe("1–50 of 1,234 matches");
  });

  it("flips a column's sort, and starts another ascending", () => {
    expect(flipSort({ key: "sent", dir: "desc" }, "sent")).toEqual({ key: "sent", dir: "asc" });
    expect(flipSort({ key: "sent", dir: "asc" }, "sent")).toEqual({ key: "sent", dir: "desc" });
    expect(flipSort({ key: "sent", dir: "desc" }, "title")).toEqual({ key: "title", dir: "asc" });
  });

  it("cycles a column's sort on, the other way, and off", () => {
    expect(nextSort(null, "n")).toEqual({ key: "n", dir: "asc" });
    expect(nextSort({ key: "n", dir: "asc" }, "n")).toEqual({ key: "n", dir: "desc" });
    expect(nextSort({ key: "n", dir: "desc" }, "n")).toBeNull();
    expect(nextSort({ key: "n", dir: "desc" }, "title")).toEqual({ key: "title", dir: "asc" });
  });

  it("counts notes with separators", () => {
    expect(noteCount(1)).toBe("1 note");
    expect(noteCount(1234)).toBe("1,234 notes");
  });
});

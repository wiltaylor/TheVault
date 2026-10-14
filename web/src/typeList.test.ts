import { beforeEach, describe, expect, it } from "vitest";
import type { FieldDef, Row, TypeDef } from "./api";
import {
  TYPE_VIEWS_KEY,
  listColumns,
  loadTypeView,
  nextSort,
  noteCount,
  saveTypeView,
  sortRows,
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

const row = (title: string, fields: Row["fields"] = {}): Row => ({ id: title, type: "book", title, fields });

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

  it("queries by title, or by relevance with search text", () => {
    expect(typeQuery("book", "  ")).toBe("from book sort title asc");
    expect(typeQuery("book", "dune")).toBe('from book search "dune"');
    expect(typeQuery("book", "dune ")).toBe('from book search "dune "');
    expect(typeQuery("book", 'say "hi"')).toBe('from book search "say \\"hi\\""');
  });

  it("sorts a column both ways with empty values last, and cycles off", () => {
    const rows = [row("b", { n: 2 }), row("a", { n: 10 }), row("c", {}), row("d", { n: 1 })];
    expect(sortRows(rows, { key: "n", dir: "asc" }).map((r) => r.title)).toEqual(["d", "b", "a", "c"]);
    expect(sortRows(rows, { key: "n", dir: "desc" }).map((r) => r.title)).toEqual(["a", "b", "d", "c"]);
    expect(sortRows(rows, { key: "title", dir: "asc" }).map((r) => r.title)).toEqual(["a", "b", "c", "d"]);
    expect(sortRows(rows, null)).toEqual(rows);
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

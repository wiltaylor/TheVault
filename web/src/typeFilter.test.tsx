// A type filter in a note picker's text, `<type>/<text>`: reading it, the search it makes, and the
// two pickers that take it, the `@` picker in the content editor and a card block's note search.

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type Block, type Page, type Row, type TypeDef, api } from "./api";
import { NoteCardBlock } from "./blocks/NoteCardBlock";
import { ContentEditor, type EditorHandle } from "./editor/ContentEditor";
import { reloadDefinitions } from "./session";
import { matchType, readTypeFilter, searchNotes } from "./typeFilter";

// jsdom lays nothing out; the editor's menus place themselves by the caret's rectangle.
if (!Element.prototype.getClientRects) Element.prototype.getClientRects = () => [] as unknown as DOMRectList;
if (!Element.prototype.scrollIntoView) Element.prototype.scrollIntoView = () => {};
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

const type = (name: string, label: string, plural: string, icon = "file"): TypeDef => ({
  name,
  label,
  plural,
  description: null,
  icon,
  colour: null,
  fields: [],
  templates: [],
});

const TYPES = [
  type("person", "Person", "People", "user"),
  type("personal_goal", "Personal goal", "Personal goals"),
  type("project", "Project", "Projects", "folder"),
  type("book", "Book", "Books", "book"),
];

describe("readTypeFilter", () => {
  const read = (raw: string) => {
    const f = readTypeFilter(raw, TYPES);
    return { type: f.type?.name ?? null, text: f.text, suggestions: f.suggestions.map((t) => t.name), problem: f.problem };
  };

  it("takes a type by its name, label or plural, case ignored", () => {
    expect(read("person/sam")).toEqual({ type: "person", text: "sam", suggestions: [], problem: null });
    expect(read("Book/dune")).toMatchObject({ type: "book", text: "dune" });
    expect(read("PEOPLE/sam")).toMatchObject({ type: "person", text: "sam" });
    expect(read("personal goal/run")).toMatchObject({ type: "personal_goal", text: "run" });
  });

  it("prefers an exact match to a longer type it starts", () => {
    // "person" starts Personal goal too, but names Person exactly.
    expect(read("person/").type).toBe("person");
    expect(matchType(TYPES, "person")).toEqual({ type: TYPES[0] });
  });

  it("takes the one type a prefix starts", () => {
    expect(read("pers/sam")).toMatchObject({ type: null, problem: "“pers” matches Person, Personal goal: type more" });
    expect(read("proj/x")).toMatchObject({ type: "project", text: "x" });
    expect(read("peo/")).toMatchObject({ type: "person", text: "" });
    expect(read("bo/")).toMatchObject({ type: "book", text: "" });
  });

  it("sets no filter for an ambiguous prefix or a word that names no type, and says why", () => {
    expect(read("p/sam")).toEqual({ type: null, text: "p/sam", suggestions: [], problem: "“p” matches Person, Personal goal, Project: type more" });
    expect(read("pe/sam")).toMatchObject({ type: null, text: "pe/sam", problem: "“pe” matches Person, Personal goal: type more" });
    expect(read("zz/sam")).toEqual({ type: null, text: "zz/sam", suggestions: [], problem: "No type matches “zz”" });
  });

  it("lets spaces stand round the slash", () => {
    expect(read("  person  /  sam  ")).toMatchObject({ type: "person", text: "sam" });
    expect(read("project / ")).toMatchObject({ type: "project", text: "" });
  });

  it("splits at the first slash only, and searches a title with a slash whole when no type matches", () => {
    expect(read("book/a/b")).toMatchObject({ type: "book", text: "a/b" });
    expect(read("TCP/IP")).toMatchObject({ type: null, text: "TCP/IP", problem: "No type matches “TCP”" });
    expect(read("/notes")).toEqual({ type: null, text: "/notes", suggestions: [], problem: null });
  });

  it("offers the types the text starts until there is a slash", () => {
    expect(read("p").suggestions).toEqual(["person", "personal_goal", "project"]);
    expect(read("Peo").suggestions).toEqual(["person"]);
    expect(read("sam").suggestions).toEqual([]);
    expect(read("").suggestions).toEqual([]);
    expect(read("p").type).toBeNull();
  });
});

describe("searchNotes", () => {
  afterEach(() => vi.restoreAllMocks());
  const page = (rows: Row[]): Page => ({ rows, next: null, total: rows.length });

  it("asks the server for one type's notes, so a page is never filtered short", async () => {
    const query = vi.spyOn(api, "query").mockResolvedValue(page([]));
    const search = vi.spyOn(api, "search").mockResolvedValue(page([]));
    await searchNotes("person", "sam \"s\"", 8);
    expect(query).toHaveBeenLastCalledWith('from person search "sam \\"s\\"" select title', null, 8);
    await searchNotes("person", " ", 8);
    expect(query).toHaveBeenLastCalledWith("from person sort updated desc select title", null, 8);
    await searchNotes(null, "sam", 8);
    expect(search).toHaveBeenLastCalledWith("sam", null, 8);
    expect(await searchNotes(null, "", 8)).toEqual([]);
    expect(search).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------------------------
// The pickers

const SAM: Row = { id: "01JB8Z4K2MQ7V3X9T5R6W8Y2Z1", type: "person", title: "Sam Jones", fields: {} };
const SAMPLE: Row = { id: "01JB8Z4K2MQ7V3X9T5R6W8Y2Z2", type: "book", title: "Sample book", fields: {} };

describe("the pickers' type filter", () => {
  let query: ReturnType<typeof vi.spyOn>;
  let search: ReturnType<typeof vi.spyOn>;
  beforeEach(async () => {
    vi.spyOn(api, "types").mockResolvedValue(TYPES);
    vi.spyOn(api, "taxonomies").mockResolvedValue([]);
    vi.spyOn(api, "agents").mockResolvedValue({ agents: [], tools: [] });
    vi.spyOn(api, "note").mockImplementation(async (id: string) => ({ id, type: "person", version: "v1", wcl: "", note: { fields: { title: SAM.title }, blocks: [] } }));
    search = vi.spyOn(api, "search").mockResolvedValue({ rows: [SAM, SAMPLE], next: null, total: 2 });
    query = vi.spyOn(api, "query").mockImplementation(async (q: string) => ({
      rows: q.startsWith("from person") && !q.includes("zed") ? [SAM] : [],
      next: null,
      total: 0,
    }));
    await reloadDefinitions();
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  async function mountEditor() {
    const onChange = vi.fn();
    let handle!: EditorHandle;
    const r = render(() => <ContentEditor blocks={[{ kind: "paragraph", fields: { text: "" } }] as Block[]} label="Body" onChange={onChange} onReady={(h) => (handle = h)} />);
    handle.editor.commands.focus("end");
    // The editor takes focus on its next frame; the picker takes it after that.
    await new Promise((done) => setTimeout(done, 50));
    handle.editor.commands.insertMentionPicker("@");
    return { ...r, onChange };
  }

  it("offers types as filters in the @ picker, and choosing one types <type>/ and keeps it open", async () => {
    const r = await mountEditor();
    const input = (await r.findByRole("combobox")) as HTMLInputElement;
    fireEvent.input(input, { target: { value: "pe" } });
    const rows = await r.findAllByTestId("mention-filter");
    expect(rows.map((x) => x.dataset.type)).toEqual(["person", "personal_goal"]);
    expect(rows[0]).toHaveTextContent("Personfilter by type");
    // Enter still takes the best note found; the filters sit above it.
    await waitFor(() => expect(r.getAllByTestId("mention-result")[0]).toHaveAttribute("aria-selected", "true"));
    fireEvent.keyDown(input, { key: "ArrowUp" });
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(r.getAllByTestId("mention-filter")[0]).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(input.value).toBe("person/");
    expect(r.getByTestId("mention-picker")).toBeInTheDocument();
    expect(await r.findByTestId("mention-type-filter")).toHaveTextContent("Person");
    // `person/` lists people, through the server, most recently changed first.
    await waitFor(() => expect(r.getAllByTestId("mention-result").map((x) => x.textContent)).toEqual(["Sam Jones Person"]));
    expect(query).toHaveBeenLastCalledWith("from person sort updated desc select title", null, 8);
  });

  it("searches one type for <type>/<text>, offers New of only that type, and picks a note", async () => {
    const r = await mountEditor();
    const input = (await r.findByRole("combobox")) as HTMLInputElement;
    fireEvent.input(input, { target: { value: "people / sa" } });
    await waitFor(() => expect(query).toHaveBeenLastCalledWith('from person search "sa" select title', null, 8));
    await waitFor(() => expect(r.getAllByTestId("mention-new").map((x) => x.dataset.type)).toEqual(["person"]));
    expect(r.queryByTestId("mention-choose-type")).toBeNull();
    expect(r.getByTestId("mention-type-filter").dataset.type).toBe("person");
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(r.queryByTestId("mention-picker")).toBeNull());
    expect(r.onChange.mock.lastCall![0][0].fields.text).toBe(`[[${SAM.id}]]`);
    expect(search).not.toHaveBeenCalled();
  });

  it("says why a slash sets no filter, searches the text whole, and drops the filter with the slash", async () => {
    const r = await mountEditor();
    const input = (await r.findByRole("combobox")) as HTMLInputElement;
    fireEvent.input(input, { target: { value: "p/sa" } });
    expect(await r.findByTestId("mention-filter-hint")).toHaveTextContent("“p” matches Person, Personal goal, Project: type more");
    await waitFor(() => expect(search).toHaveBeenLastCalledWith("p/sa", null, 8));
    expect(r.queryByTestId("mention-type-filter")).toBeNull();
    fireEvent.input(input, { target: { value: "zz/" } });
    expect(await r.findByTestId("mention-filter-hint")).toHaveTextContent("No type matches “zz”");
    fireEvent.input(input, { target: { value: "book/" } });
    expect(await r.findByTestId("mention-filter-hint")).toHaveTextContent("No book yet");
    fireEvent.input(input, { target: { value: "book" } });
    await waitFor(() => expect(r.queryByTestId("mention-type-filter")).toBeNull());
  });

  it("filters a card's note search the same way, with the type as a badge", async () => {
    const onChange = vi.fn();
    const r = render(() => <NoteCardBlock block={{ kind: "card", fields: {} }} onChange={onChange} />);
    const input = (await r.findByRole("combobox", { name: "Note to show as a card" })) as HTMLInputElement;
    fireEvent.input(input, { target: { value: "per" } });
    const options = async () => (await r.findAllByRole("option")).map((o) => o.textContent);
    await waitFor(async () => expect(await options()).toEqual(["Personfilter by type", "Personal goalfilter by type", "Sam JonesPerson", "Sample bookBook"]));
    fireEvent.click(r.getAllByRole("option")[0]!);
    expect(input.value).toBe("person/");
    expect(document.activeElement).toBe(input);
    expect(await r.findByTestId("combo-badge")).toHaveTextContent("Person");
    await waitFor(async () => expect(await options()).toEqual(["Sam JonesPerson"]));
    fireEvent.input(input, { target: { value: "person/zed" } });
    await waitFor(async () => expect(await options()).toEqual(["No person matches “zed”"]));
    expect(query).toHaveBeenLastCalledWith('from person search "zed" select title', null, 20);
    fireEvent.input(input, { target: { value: "pe/sam" } });
    await waitFor(async () => expect(await options()).toEqual(["“pe” matches Person, Personal goal: type more", "Sam JonesPerson", "Sample bookBook"]));
    expect(r.queryByTestId("combo-badge")).toBeNull();
    fireEvent.input(input, { target: { value: "person/sam" } });
    await waitFor(async () => expect(await options()).toEqual(["Sam JonesPerson"]));
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith({ kind: "card", fields: { note: SAM.id } });
  });
});

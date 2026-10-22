import { MemoryRouter, Route } from "@solidjs/router";
import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ListPage, Note, Page, Row, TypeDef } from "../api";
import { TYPE_PREFS_KEY } from "../typeList";

// A type's list page (lists L2 to L4): a page at a time by cursor, sorted on the server by the
// type's sort or the user's column, in the view the type or the user chose.

const query = vi.fn();
const note = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, query, note } };
});

const types: TypeDef[] = [];
vi.mock("../session", async (orig) => {
  const real = await orig<typeof import("../session")>();
  return { ...real, typeDef: (name: string) => types.find((t) => t.name === name) };
});

const { TypeNotes } = await import("./TypeList");

function bookType(name: string, list_page?: ListPage): TypeDef {
  return {
    name,
    label: "Book",
    plural: "Books",
    description: null,
    icon: null,
    colour: null,
    fields: [
      { name: "pages", label: "Pages", description: null, kind: "int", required: false, computed: false, multiple: false, compound: false },
    ],
    templates: [
      { name: "default", label: null, tabs: [{ title: "Main", items: [{ slot: "pages", label: null }] }] },
      { name: "card", label: null, tabs: [{ title: "Card", items: [{ slot: "pages", label: null }] }] },
    ],
    ...(list_page ? { list_page } : {}),
  };
}

/** A server holding `count` notes of every type, paged by an offset cursor. */
function serve(count: number) {
  query.mockImplementation(async (q: string, cursor: string | null, size: number): Promise<Page> => {
    const type = /^from (\w+)/.exec(q)![1]!;
    const from = cursor ? Number(cursor.slice(1)) : 0;
    const to = Math.min(count, from + size);
    const rows: Row[] = Array.from({ length: to - from }, (_, k) => ({
      id: `${type}-${from + k + 1}`,
      type,
      title: `Note ${from + k + 1}`,
      fields: { pages: from + k + 1 },
    }));
    return { rows, total: count, next: to < count ? `c${to}` : null };
  });
  note.mockImplementation(
    async (id: string): Promise<Note> => ({ id, type: "", version: "v1", wcl: "", note: { fields: { title: id }, blocks: [] } }),
  );
}

function mount(type: TypeDef) {
  types.splice(0, types.length, type);
  const r = render(() => (
    <MemoryRouter>
      <Route path="*" component={() => <TypeNotes type={type} />} />
    </MemoryRouter>
  ));
  const status = () => r.getByTestId("type-list-status").textContent;
  const button = (name: string) => r.getByRole("button", { name }) as HTMLButtonElement;
  const lastQuery = () => query.mock.calls.at(-1) as [string, string | null, number];
  return { ...r, status, button, lastQuery };
}

beforeEach(() => localStorage.clear());

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("type list paging", () => {
  it("pages through 120 notes 50 at a time with First, Previous and Next", async () => {
    serve(120);
    const m = mount(bookType("book", { page_size: 50 }));
    await waitFor(() => expect(m.status()).toBe("1–50 of 120"));
    expect(m.getAllByTestId("note-card")).toHaveLength(50);
    expect(m.lastQuery()).toEqual(["from book sort updated desc", null, 50]);
    expect(m.button("First").disabled).toBe(true);
    expect(m.button("Previous").disabled).toBe(true);

    fireEvent.click(m.button("Next"));
    await waitFor(() => expect(m.status()).toBe("51–100 of 120"));
    expect(m.lastQuery()).toEqual(["from book sort updated desc", "c50", 50]);
    expect(m.getAllByTestId("query-title")[0]!.textContent).toBe("Note 51");

    fireEvent.click(m.button("Previous"));
    await waitFor(() => expect(m.status()).toBe("1–50 of 120"));
    expect(m.getAllByTestId("query-title")[0]!.textContent).toBe("Note 1");

    fireEvent.click(m.button("Next"));
    await waitFor(() => expect(m.status()).toBe("51–100 of 120"));
    fireEvent.click(m.button("Next"));
    await waitFor(() => expect(m.status()).toBe("101–120 of 120"));
    expect(m.getAllByTestId("note-card")).toHaveLength(20);
    expect(m.button("Next").disabled).toBe(true);

    fireEvent.click(m.button("First"));
    await waitFor(() => expect(m.status()).toBe("1–50 of 120"));
    expect(m.getAllByTestId("query-title")[0]!.textContent).toBe("Note 1");
    // No page ever asked for more than the page size.
    expect(query.mock.calls.every((c) => c[2] === 50)).toBe(true);
  });

  it("shows no pager when one page holds every note", async () => {
    serve(3);
    const m = mount(bookType("book"));
    await waitFor(() => expect(m.status()).toBe("3 notes"));
    expect(m.queryByRole("navigation", { name: "Pages" })).toBeNull();
  });
});

describe("type list sort", () => {
  it("sends the type's sort and page size to the query", async () => {
    serve(10);
    const m = mount(bookType("book", { sort: "pages desc", page_size: 25 }));
    await waitFor(() => expect(m.status()).toBe("10 notes"));
    expect(m.lastQuery()).toEqual(["from book sort pages desc", null, 25]);
  });

  it("re-queries with a clicked column's sort from page 1, and remembers it", async () => {
    serve(120);
    const m = mount(bookType("book", { view: "list", sort: "pages desc" }));
    await waitFor(() => expect(m.status()).toBe("1–50 of 120"));
    const pages = m.getByRole("columnheader", { name: /Pages/ });
    expect(pages.getAttribute("aria-sort")).toBe("descending");
    fireEvent.click(m.button("Next"));
    await waitFor(() => expect(m.status()).toBe("51–100 of 120"));

    fireEvent.click(m.getByRole("button", { name: /^Pages/ }));
    await waitFor(() => expect(m.lastQuery()).toEqual(["from book sort pages asc", null, 50]));
    await waitFor(() => expect(m.status()).toBe("1–50 of 120"));
    expect(m.getByRole("columnheader", { name: /Pages/ }).getAttribute("aria-sort")).toBe("ascending");

    fireEvent.click(m.getByRole("button", { name: /^Title/ }));
    await waitFor(() => expect(m.lastQuery()).toEqual(["from book sort title asc", null, 50]));
    expect(JSON.parse(localStorage.getItem(TYPE_PREFS_KEY)!)).toEqual({ book: { sort: { key: "title", dir: "asc" } } });
  });
});

describe("type list views", () => {
  it("opens an email in the details view the type sets, each note expanded", async () => {
    serve(2);
    const m = mount(bookType("email", { view: "details", sort: "pages desc", page_size: 50 }));
    await waitFor(() => expect(m.getAllByTestId("type-detail")).toHaveLength(2));
    expect(m.button("Details").getAttribute("aria-pressed")).toBe("true");
    expect(m.queryByTestId("note-card")).toBeNull();
    const titles = m.getAllByTestId("query-title");
    expect(titles.map((t) => t.textContent)).toEqual(["Note 1", "Note 2"]);
    expect(titles[0]!.getAttribute("href")).toBe("/notes/email-1");
    expect(m.lastQuery()).toEqual(["from email sort pages desc", null, 50]);
    expect(m.queryByRole("button", { name: "Reset to type default" })).toBeNull();
  });

  it("a remembered view wins until Reset to type default", async () => {
    serve(2);
    localStorage.setItem(TYPE_PREFS_KEY, JSON.stringify({ email: { view: "cards" } }));
    const m = mount(bookType("email", { view: "details" }));
    await waitFor(() => expect(m.getAllByTestId("note-card")).toHaveLength(2));
    expect(m.button("Cards").getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(m.button("Reset to type default"));
    await waitFor(() => expect(m.getAllByTestId("type-detail")).toHaveLength(2));
    expect(m.button("Details").getAttribute("aria-pressed")).toBe("true");
    expect(JSON.parse(localStorage.getItem(TYPE_PREFS_KEY)!)).toEqual({});
    expect(m.queryByRole("button", { name: "Reset to type default" })).toBeNull();

    // Choosing a view again remembers it; choosing the type's own forgets it.
    fireEvent.click(m.button("List"));
    await waitFor(() => expect(m.getAllByTestId("note-row")).toHaveLength(2));
    expect(JSON.parse(localStorage.getItem(TYPE_PREFS_KEY)!)).toEqual({ email: { view: "list" } });
    fireEvent.click(m.button("Details"));
    expect(JSON.parse(localStorage.getItem(TYPE_PREFS_KEY)!)).toEqual({});
  });
});

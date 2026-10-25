import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Block, Note, TypeDef } from "../api";

// A `card` block (docs/vault-format.md, "card"): a note as a card through its type's `card`
// template, that opens the note when clicked, and a search to choose the note while editing.

const note = vi.fn();
const search = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, note, search } };
});

const followLink = vi.fn(() => true);
vi.mock("../workspace/events", async (orig) => {
  const real = await orig<typeof import("../workspace/events")>();
  return { ...real, followLink };
});

const types: TypeDef[] = [];
vi.mock("../session", async (orig) => {
  const real = await orig<typeof import("../session")>();
  return { ...real, typeDef: (name: string) => types.find((t) => t.name === name) };
});

const { ApiError } = await import("../api");
const { blocksToDoc, docToBlocks } = await import("../editor/doc");
const { NoteCardBlock } = await import("./NoteCardBlock");

const BOOK = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const LOOSE = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC";

function type(name: string, card: boolean): TypeDef {
  return {
    name,
    label: name === "book" ? "Book" : "Loose",
    plural: "",
    description: null,
    icon: "book",
    colour: "green",
    fields: [{ name: "pages", label: "Pages", description: null, kind: "int", required: false, computed: false, multiple: false, compound: false }],
    templates: [
      { name: "default", label: null, tabs: [{ title: "Main", items: [{ slot: "title", label: null }, { slot: "pages", label: null }] }] },
      ...(card ? [{ name: "card", label: null, tabs: [{ title: "Card", items: [{ slot: "pages", label: null }] }] }] : []),
    ],
  };
}

types.push(type("book", true), type("loose", false));

note.mockImplementation(async (id: string): Promise<Note> => {
  if (id === BOOK) return { id, type: "book", version: "v1", wcl: "", note: { fields: { title: "Dune", pages: 412 }, blocks: [] } };
  if (id === LOOSE) return { id, type: "loose", version: "v1", wcl: "", note: { fields: { title: "Scrap", pages: 3 }, blocks: [] } };
  throw new ApiError(404, { error: "not_found", message: "no such note" });
});

const card = (id?: string): Block => ({ kind: "card", fields: id ? { note: id } : {} });

describe("a card block", () => {
  afterEach(() => {
    cleanup();
    followLink.mockClear();
  });

  it("shows the note's title linking to it, then its type's card template", async () => {
    const { findByTestId, container } = render(() => <NoteCardBlock block={card(BOOK)} />);
    const title = await findByTestId("note-card-title");
    await waitFor(() => expect(title.textContent).toBe("Dune"));
    expect(title.getAttribute("href")).toBe(`/notes/${BOOK}`);
    expect(container.querySelector(".query-slot[data-field='pages']")?.textContent).toContain("412");
    const box = container.querySelector<HTMLElement>("[data-testid='note-card']")!;
    expect(box.style.getPropertyValue("--note-card")).toBe("var(--palette-green)");
  });

  it("shows only the title for a type with no card template, not its default one", async () => {
    const { findByTestId, container } = render(() => <NoteCardBlock block={card(LOOSE)} />);
    await waitFor(async () => expect((await findByTestId("note-card-title")).textContent).toBe("Scrap"));
    expect(container.querySelector(".query-slot")).toBeNull();
  });

  it("opens the note on a click anywhere on the card, in the background with Ctrl", async () => {
    const { findByTestId, container } = render(() => <NoteCardBlock block={card(BOOK)} />);
    await findByTestId("note-card-title");
    const box = container.querySelector("[data-testid='note-card']")!;
    fireEvent.click(container.querySelector(".query-slot")!);
    expect(followLink).toHaveBeenLastCalledWith(`/notes/${BOOK}`, box, false);
    fireEvent.click(box, { ctrlKey: true });
    expect(followLink).toHaveBeenLastCalledWith(`/notes/${BOOK}`, box, true);
    expect(followLink).toHaveBeenCalledTimes(2);
  });

  it("shows a missing note as missing, as a broken mention does", async () => {
    const { findByText, container } = render(() => <NoteCardBlock block={card("01JB8Z4K2MQ7V3X9T5R6W8Y2ZZ")} />);
    await findByText("Missing note");
    expect(container.querySelector("[data-missing='true']")).not.toBeNull();
    expect(container.querySelector("a")).toBeNull();
  });

  it("is a note search while being edited with no note, and choosing a note sets it", async () => {
    search.mockResolvedValue({ rows: [{ id: BOOK, type: "book", title: "Dune", fields: {} }], total: 1, next: null });
    const [b, setB] = createSignal(card());
    const onChange = vi.fn((next: Block) => setB(next));
    const onRemove = vi.fn();
    const { findByRole, findByText, findByTestId } = render(() => <NoteCardBlock block={b()} onChange={onChange} onRemove={onRemove} />);
    const box = await findByRole("combobox", { name: "Note to show as a card" });
    fireEvent.focus(box);
    fireEvent.input(box, { target: { value: "dune" } });
    fireEvent.click(await findByText("Dune"));
    expect(search).toHaveBeenCalledWith("dune", null, expect.any(Number));
    expect(onChange).toHaveBeenCalledWith(card(BOOK));
    await waitFor(async () => expect((await findByTestId("note-card-title")).textContent).toBe("Dune"));
    expect(onRemove).not.toHaveBeenCalled();
  });

  it("takes a new card out again on Escape in its search", async () => {
    const onRemove = vi.fn();
    const { findByRole } = render(() => <NoteCardBlock block={card()} onChange={() => {}} onRemove={onRemove} />);
    fireEvent.keyDown(await findByRole("combobox"), { key: "Escape" });
    expect(onRemove).toHaveBeenCalledTimes(1);
  });

  it("changes its note with Change, and Escape there keeps the note it had", async () => {
    const onChange = vi.fn();
    const { findByLabelText, findByRole, findByTestId, queryByRole } = render(() => (
      <NoteCardBlock block={card(BOOK)} onChange={onChange} onRemove={() => {}} />
    ));
    fireEvent.click(await findByLabelText("Change note"));
    fireEvent.keyDown(await findByRole("combobox"), { key: "Escape" });
    expect(queryByRole("combobox")).toBeNull();
    await findByTestId("note-card-title");
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("a card block in the editor document", () => {
  it("rides along whole, and is dropped while it has no note", () => {
    const blocks: Block[] = [card(BOOK), { kind: "paragraph", fields: { text: "after" } }];
    expect(docToBlocks(blocksToDoc(blocks))).toEqual(blocks);
    expect(docToBlocks(blocksToDoc([card(), { kind: "paragraph", fields: { text: "after" } }]))).toEqual([
      { kind: "paragraph", fields: { text: "after" } },
    ]);
  });
});

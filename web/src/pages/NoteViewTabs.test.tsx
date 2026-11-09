import { MemoryRouter, Route } from "@solidjs/router";
import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Note, Tab, TypeDef } from "../api";

// A template tab's `when` on the note page (D14): evaluated live on the note as edited.

const note = vi.fn();
const updateNote = vi.fn();
const dispatched = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, note, updateNote, dispatched, backlinks: () => Promise.resolve([]) } };
});

const types: TypeDef[] = [];
vi.mock("../session", async (orig) => {
  const real = await orig<typeof import("../session")>();
  return { ...real, typeDef: (name: string) => types.find((t) => t.name === name) };
});

const { NoteView } = await import("./NoteView");

const text = (t: string) => ({ block: { kind: "paragraph", labels: [], fields: { text: t } } });

function taskType(tabs: Tab[]): TypeDef {
  return {
    name: "task",
    label: "Task",
    plural: "Tasks",
    description: null,
    icon: null,
    colour: null,
    fields: [
      { name: "done", label: "Done", description: null, kind: "bool", required: false, computed: false, multiple: false, compound: false },
      {
        name: "stage",
        label: "Stage",
        description: null,
        kind: "select",
        options: ["idea", "draft"],
        required: false,
        computed: false,
        multiple: false,
        compound: false,
      },
    ],
    templates: [{ name: "default", label: null, tabs }],
  };
}

const TABS: Tab[] = [
  { title: "Main", items: [{ slot: "title", label: null }, { slot: "stage", label: null }] },
  { title: "Details", items: [text("Details text")] },
  { title: "Review", when: 'stage == "draft"', items: [text("Review text")] },
  { title: "Open", when: "!done", items: [{ slot: "done", label: null }] },
];

const ID = "01JB8Z4K2MQ7V3X9T5R6W8Y2T1";

function mount(tabs: Tab[], fields: Record<string, unknown>) {
  types.splice(0, types.length, taskType(tabs));
  const stored: Note = {
    id: ID,
    type: "task",
    version: "v1",
    wcl: "",
    note: { fields: { title: "Ship it", ...fields } as Note["note"]["fields"], blocks: [] },
  };
  note.mockResolvedValue(stored);
  if (!dispatched.getMockImplementation()) dispatched.mockResolvedValue({ threads: [] });
  updateNote.mockReturnValue(new Promise(() => {}));
  const r = render(() => (
    <MemoryRouter>
      <Route path="*" component={() => <NoteView id={ID} />} />
    </MemoryRouter>
  ));
  const tabTitles = () => r.queryAllByRole("tab").map((t) => t.textContent);
  const selected = () => r.queryAllByRole("tab").find((t) => t.getAttribute("aria-selected") === "true")?.textContent;
  return { ...r, tabTitles, selected };
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  dispatched.mockReset();
});

describe("conditional tabs on the note page", () => {
  it("puts one send-to-agent robot icon beside archive and delete", async () => {
    const m = mount(TABS, {});
    const send = await m.findByRole("button", { name: "Send to agent" });
    expect(m.getAllByRole("button", { name: "Send to agent" })).toHaveLength(1);
    expect(send).toHaveClass("icon-button");
    expect(send).toHaveAttribute("title", "Send to agent");
    expect(send.querySelector("svg.robot-glyph")).not.toBeNull();
    expect(send.parentElement).toBe(m.getByRole("button", { name: "Archive" }).parentElement);
    expect(send.parentElement).toBe(m.getByRole("button", { name: "Delete" }).parentElement);
    fireEvent.click(send);
    expect(await m.findByRole("dialog")).toHaveTextContent("Send this note to an agent");
  });
  it("leaves out a tab whose when is false", async () => {
    const m = mount(TABS, { stage: "idea", done: false });
    await waitFor(() => expect(m.tabTitles()).toEqual(["Main", "Details", "Open"]));
  });

  it("shows a tab as soon as the field it reads changes, before any save", async () => {
    const m = mount(TABS, { stage: "idea", done: false });
    await waitFor(() => expect(m.tabTitles()).toEqual(["Main", "Details", "Open"]));
    const stage = m.getByRole("combobox", { name: "Stage" }) as HTMLSelectElement;
    fireEvent.change(stage, { target: { value: "draft" } });
    expect(m.tabTitles()).toEqual(["Main", "Details", "Review", "Open"]);
    expect(updateNote).not.toHaveBeenCalled();
    fireEvent.change(stage, { target: { value: "idea" } });
    expect(m.tabTitles()).toEqual(["Main", "Details", "Open"]);
  });

  it("moves to the first tab that shows when the open tab is hidden", async () => {
    const m = mount(TABS, { stage: "draft", done: false });
    await waitFor(() => expect(m.tabTitles()).toEqual(["Main", "Details", "Review", "Open"]));
    fireEvent.click(m.getByRole("tab", { name: "Open" }));
    expect(m.selected()).toBe("Open");
    fireEvent.click(m.getByRole("checkbox", { name: "Done" }));
    expect(m.tabTitles()).toEqual(["Main", "Details", "Review"]);
    expect(m.selected()).toBe("Main");
    expect(m.queryByRole("checkbox", { name: "Done" })).toBeNull();
  });

  it("shows the first tab when every tab is hidden, so the note never renders empty", async () => {
    const hidden: Tab[] = [
      { title: "First", when: "done", items: [text("First text")] },
      { title: "Second", when: "done", items: [text("Second text")] },
    ];
    const m = mount(hidden, { done: false });
    await waitFor(() => expect(m.getByText("First text")).toBeTruthy());
    expect(m.tabTitles()).toEqual([]);
    expect(m.queryByText("Second text")).toBeNull();
  });
});

describe("conditional slots on the note page", () => {
  const column = (slots: { field: string; when?: string }[]) => ({
    kind: "column",
    blocks: slots.map((s) => ({ kind: "slot", labels: [s.field], ...(s.when ? { fields: { when: s.when } } : {}) })),
  });
  const SLOTS: Tab[] = [
    {
      title: "Main",
      items: [
        { slot: "stage", label: null, when: "!done" },
        { block: { kind: "columns", blocks: [column([{ field: "done" }]), column([{ field: "title" }])] } },
      ],
    },
  ];

  it("shows a slot only while its when holds, as the note is edited", async () => {
    const m = mount(SLOTS, { stage: "idea", done: false });
    await waitFor(() => expect(m.getByRole("combobox", { name: "Stage" })).toBeTruthy());
    fireEvent.click(m.getByRole("checkbox", { name: "Done" }));
    expect(m.queryByRole("combobox", { name: "Stage" })).toBeNull();
    expect(updateNote).not.toHaveBeenCalled();
  });

  it("reads a when on a slot in a column too", async () => {
    const tabs: Tab[] = [
      {
        title: "Main",
        items: [
          { slot: "done", label: null },
          { block: { kind: "columns", blocks: [column([{ field: "stage", when: "done" }]), column([{ field: "title" }])] } },
        ],
      },
    ];
    const m = mount(tabs, { stage: "idea", done: false });
    await waitFor(() => expect(m.getByRole("checkbox", { name: "Done" })).toBeTruthy());
    expect(m.queryByRole("combobox", { name: "Stage" })).toBeNull();
    fireEvent.click(m.getByRole("checkbox", { name: "Done" }));
    expect(m.getByRole("combobox", { name: "Stage" })).toBeTruthy();
  });
});

describe("sending the note to an agent (TV-78)", () => {
  it("has a Send to agent button and a robot for each thread about the note", async () => {
    dispatched.mockResolvedValue({
      threads: [
        { id: "01THREAD", agent: "clerk", title: "Ship it: tidy", origin: "dispatch", notes: [ID], state: "needs_you", pending: 1, question: null, last_activity: null, running: false, turn: null, started_ms: null },
      ],
    });
    const m = mount(TABS, { stage: "idea", done: false });
    await waitFor(() => expect(m.getByRole("button", { name: "Send to agent" })).toBeTruthy());
    await waitFor(() => expect(m.getByTestId("agent-robot").getAttribute("data-state")).toBe("needs_you"));
    expect(dispatched).toHaveBeenCalledWith({ notes: [ID] });
    expect(m.getByTestId("agent-robot").getAttribute("href")).toBe("/agents/clerk?thread=01THREAD");
  });
});

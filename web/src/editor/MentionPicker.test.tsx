// The mention picker's "Ask <Label>" entries (agents G16): typing `@` lists, before the notes,
// the agents a note may call, and choosing one writes the call as text.

import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { callableAgents, refreshAgents } from "../agents";
import { type AgentSummary, type Block, api } from "../api";
import { ContentEditor, type EditorHandle } from "./ContentEditor";

// jsdom lays nothing out; the editor's menus place themselves by the caret's rectangle.
if (!Element.prototype.getClientRects) Element.prototype.getClientRects = () => [] as unknown as DOMRectList;
if (!Element.prototype.scrollIntoView) Element.prototype.scrollIntoView = () => {};
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

const AGENTS: AgentSummary[] = [
  { name: "assistant", label: "Assistant", built_in: true, mention: null },
  { name: "clerk", label: "Clerk", icon: "pen", built_in: false, enabled: true, mention: { types: null } },
  { name: "owl", label: "Night owl", built_in: false, enabled: true, mention: { types: ["memo"] } },
  { name: "sleepy", label: "Sleepy", built_in: false, enabled: false, mention: { types: null } },
];

beforeEach(async () => {
  vi.spyOn(api, "agents").mockResolvedValue({ agents: AGENTS, tools: [] });
  vi.spyOn(api, "search").mockResolvedValue({ rows: [], next: null } as unknown as Awaited<ReturnType<typeof api.search>>);
  await refreshAgents();
});

afterEach(() => vi.restoreAllMocks());

describe("callableAgents", () => {
  it("keeps the enabled agents the note's type may call, by the start of their name or label", () => {
    const names = (type: string | undefined, q = "") => callableAgents(AGENTS, type, q).map((a) => a.name);
    expect(names("memo")).toEqual(["clerk", "owl"]);
    expect(names("task")).toEqual(["clerk"]);
    // With no type known, only the agents every type may call.
    expect(names(undefined)).toEqual(["clerk"]);
    // A conversation calls only an agent whose trigger names its type.
    expect(names("agent_thread")).toEqual([]);
    expect(names("memo", "ni")).toEqual(["owl"]);
    expect(names("memo", "OW")).toEqual(["owl"]);
    expect(names("memo", "x")).toEqual([]);
  });
});

describe("the @ picker", () => {
  function mount(blocks: Block[]) {
    const onChange = vi.fn();
    let handle!: EditorHandle;
    const r = render(() => <ContentEditor blocks={blocks} label="Body" onChange={onChange} onReady={(h) => (handle = h)} />);
    return { ...r, onChange, editor: () => handle.editor };
  }

  it("lists Ask <Label> first and writes the call as text", async () => {
    const r = mount([{ kind: "paragraph", fields: { text: "" } }]);
    r.editor().commands.focus("end");
    r.editor().commands.insertMentionPicker("@");
    const asks = await r.findAllByTestId("mention-agent");
    // Outside a note the type is not known: only Clerk, whom every type may call.
    expect(asks.map((a) => a.textContent)).toEqual(["Ask Clerk @clerk"]);
    expect(asks[0]).toHaveAttribute("aria-selected", "true");
    fireEvent.mouseDown(asks[0]!);
    await waitFor(() => expect(r.onChange).toHaveBeenCalled());
    expect(r.onChange.mock.lastCall![0]).toEqual([{ kind: "paragraph", fields: { text: "@clerk " } }]);
    expect(r.queryByTestId("mention-picker")).toBeNull();
    r.unmount();
  });

  it("narrows the agents by what is typed, and leaves the text when given up", async () => {
    const r = mount([{ kind: "paragraph", fields: { text: "" } }]);
    r.editor().commands.focus("end");
    r.editor().commands.insertMentionPicker("@");
    const input = (await r.findByRole("combobox")) as HTMLInputElement;
    fireEvent.input(input, { target: { value: "cl" } });
    expect((await r.findAllByTestId("mention-agent")).length).toBe(1);
    fireEvent.input(input, { target: { value: "zz" } });
    await waitFor(() => expect(r.queryAllByTestId("mention-agent")).toHaveLength(0));
    fireEvent.keyDown(input, { key: "Escape" });
    await waitFor(() => expect(r.onChange).toHaveBeenCalled());
    expect(r.onChange.mock.lastCall![0]).toEqual([{ kind: "paragraph", fields: { text: "@zz" } }]);
    r.unmount();
  });
});

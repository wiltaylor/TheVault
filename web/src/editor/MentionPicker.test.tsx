// The mention picker's "Ask <Label>" entries (agents G16, G19): typing `@` lists, before the
// notes, the agents a note may call; choosing one opens the request box, and Enter commits the
// call as one link with a fresh call id.

import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { callableAgents, refreshAgents } from "../agents";
import { resetCalls } from "../agentCalls";
import { type AgentSummary, ApiError, type Block, api } from "../api";
import { ContentEditor, type EditorHandle } from "./ContentEditor";

// jsdom lays nothing out; the editor's menus place themselves by the caret's rectangle.
if (!Element.prototype.getClientRects) Element.prototype.getClientRects = () => [] as unknown as DOMRectList;
if (!Element.prototype.scrollIntoView) Element.prototype.scrollIntoView = () => {};
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

const AGENTS: AgentSummary[] = [
  { name: "assistant", label: "Assistant", built_in: true, mention: { types: null } },
  { name: "clerk", label: "Clerk", icon: "pen", built_in: false, enabled: true, mention: { types: null } },
  { name: "owl", label: "Night owl", built_in: false, enabled: true, mention: { types: ["memo"] } },
  { name: "sleepy", label: "Sleepy", built_in: false, enabled: false, mention: { types: null } },
];

beforeEach(async () => {
  resetCalls();
  vi.spyOn(api, "agents").mockResolvedValue({ agents: AGENTS, tools: [] });
  vi.spyOn(api, "agentCall").mockRejectedValue(new ApiError(404, { error: "not_found", message: "not seen" }));
  vi.spyOn(api, "search").mockResolvedValue({ rows: [], next: null } as unknown as Awaited<ReturnType<typeof api.search>>);
  await refreshAgents();
});

afterEach(() => vi.restoreAllMocks());

describe("callableAgents", () => {
  it("keeps the enabled agents the note's type may call, by the start of their name or label", () => {
    const names = (type: string | undefined, q = "") => callableAgents(AGENTS, type, q).map((a) => a.name);
    // Every enabled agent can be called; Assistant has no mention trigger and answers any type.
    expect(names("memo")).toEqual(["assistant", "clerk", "owl"]);
    expect(names("task")).toEqual(["assistant", "clerk"]);
    // With no type known, only the agents every type may call.
    expect(names(undefined)).toEqual(["assistant", "clerk"]);
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

  /** Open the picker, choose Ask Clerk, and answer the request box. */
  async function ask(r: ReturnType<typeof mount>) {
    r.editor().commands.focus("end");
    r.editor().commands.insertMentionPicker("@");
    const asks = await r.findAllByTestId("mention-agent");
    // Outside a note the type is not known: only the agents every type may call.
    expect(asks.map((a) => a.textContent)).toEqual(["Ask Assistant @assistant", "Ask Clerk @clerk"]);
    expect(asks[0]).toHaveAttribute("aria-selected", "true");
    fireEvent.mouseDown(asks[1]!);
    const box = (await r.findByRole("textbox", { name: "Ask Clerk…" })) as HTMLTextAreaElement;
    expect(box).toHaveAttribute("placeholder", "What should Clerk do?");
    expect(r.queryByTestId("mention-agent")).toBeNull();
    return box;
  }

  const CALL_TEXT = /^\[@Clerk tidy this\]\(agent:clerk\?call=[0-9A-HJKMNP-TV-Z]{26}\) $/;

  it("lists Ask <Label> first, opens the request box and commits the call in one go", async () => {
    const r = mount([{ kind: "paragraph", fields: { text: "" } }]);
    const box = await ask(r);
    // Nothing reaches the note while the request is written.
    fireEvent.input(box, { target: { value: "tidy this" } });
    for (const call of r.onChange.mock.calls) expect(JSON.stringify(call[0])).not.toContain("agent:");
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(r.queryByTestId("mention-picker")).toBeNull());
    const withCall = r.onChange.mock.calls.filter((c) => JSON.stringify(c[0]).includes("agent:"));
    expect(withCall).toHaveLength(1);
    const text = (withCall[0]![0] as Block[])[0]!.fields!.text as string;
    expect(text).toMatch(CALL_TEXT);
    // The chip shows in the editor, sending until the server has seen the call.
    expect(r.container.querySelector(".agent-call-start")).not.toBeNull();
    expect(await r.findByTestId("agent-call-sending")).toHaveTextContent("Sending…");
    r.unmount();
  });

  it("makes a fresh call id for every call, and Shift+Enter's lines stay in the call", async () => {
    const r = mount([{ kind: "paragraph", fields: { text: "" } }]);
    let box = await ask(r);
    fireEvent.input(box, { target: { value: "tidy this" } });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(r.queryByTestId("mention-picker")).toBeNull());
    box = await ask(r);
    // Shift+Enter is left to the box, which adds the line.
    expect(fireEvent.keyDown(box, { key: "Enter", shiftKey: true })).toBe(true);
    fireEvent.input(box, { target: { value: "one\ntwo" } });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(r.queryByTestId("mention-picker")).toBeNull());
    const text = (r.onChange.mock.lastCall![0] as Block[])[0]!.fields!.text as string;
    const ids = [...text.matchAll(/call=([0-9A-Z]{26})/g)].map((m) => m[1]);
    expect(ids).toHaveLength(2);
    expect(ids[0]).not.toEqual(ids[1]);
    expect(text).toContain("[@Clerk one\ntwo](agent:clerk?call=");
    r.unmount();
  });

  it("leaves nothing on Escape or an empty Enter", async () => {
    for (const key of ["Escape", "Enter"]) {
      const r = mount([{ kind: "paragraph", fields: { text: "Hi " } }]);
      const box = await ask(r);
      if (key === "Escape") fireEvent.input(box, { target: { value: "tidy this" } });
      fireEvent.keyDown(box, { key });
      await waitFor(() => expect(r.queryByTestId("mention-picker")).toBeNull());
      for (const call of r.onChange.mock.calls) expect(JSON.stringify(call[0])).not.toContain("agent:");
      const last = r.onChange.mock.lastCall?.[0] as Block[] | undefined;
      if (last) expect(last[0]!.fields!.text).toBe("Hi ");
      r.unmount();
    }
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

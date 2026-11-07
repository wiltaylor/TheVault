// Send notes from a list (TV-79): rows are selected one by one or all on the page, the bar says how
// many notes one send takes and refuses more, sends all selected notes in one request through the
// Send to agent dialog, and clears the selection once sent.

import { fireEvent, render, screen } from "@solidjs/testing-library";
import { For, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { refreshAgents } from "../agents";
import { type AgentSummary, api } from "../api";
import { SelectBox, SendNotesBar, createSelection } from "./SendNotesBar";
import { MAX_NOTES } from "./SendToAgent";

const AGENTS = [{ name: "clerk", label: "Clerk", icon: null, built_in: false, enabled: true }] as AgentSummary[];
const ids = (n: number) => Array.from({ length: n }, (_, i) => `01JN${String(i).padStart(4, "0")}`);

beforeEach(async () => {
  vi.spyOn(api, "agents").mockResolvedValue({ agents: AGENTS, tools: [] });
  await refreshAgents();
});

afterEach(() => vi.restoreAllMocks());

/** A list of `notes` with a checkbox per row and the bar, as the Inbox and Search show them. */
function List(props: { notes: string[] }) {
  const [page, setPage] = createSignal(props.notes);
  const selection = createSelection(page);
  return (
    <>
      <SendNotesBar selection={selection} page={page} />
      <ul>
        <For each={page()}>
          {(id) => (
            <li>
              <SelectBox selection={selection} id={id} title={`Note ${id}`} />
            </li>
          )}
        </For>
      </ul>
      <button type="button" onClick={() => setPage((p) => p.slice(1))}>
        Drop first
      </button>
    </>
  );
}

const sendButton = () => screen.queryByRole("button", { name: /^Send \d+ notes? to agent$/ });

describe("the send bar", () => {
  it("offers Send only with notes selected, and counts them", () => {
    const r = render(() => <List notes={ids(3)} />);
    expect(sendButton()).toBeNull();
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Note 01JN0001" }));
    expect(screen.getByText("1 selected")).toBeInTheDocument();
    expect(sendButton()).toHaveTextContent("Send 1 note to agent");
    expect(screen.getByText(`Up to ${MAX_NOTES} notes at once`)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Note 01JN0002" }));
    expect(sendButton()).toHaveTextContent("Send 2 notes to agent");
    const all = screen.getByRole("checkbox", { name: "Select all on this page" }) as HTMLInputElement;
    expect(all.checked).toBe(false);
    expect(all.indeterminate).toBe(true);
    r.unmount();
  });

  it("selects all on the page and none again", () => {
    const r = render(() => <List notes={ids(3)} />);
    const all = screen.getByRole("checkbox", { name: "Select all on this page" }) as HTMLInputElement;
    fireEvent.click(all);
    expect(all.checked).toBe(true);
    expect(all.indeterminate).toBe(false);
    expect(screen.getAllByRole("checkbox", { name: /^Select Note/ }).every((b) => (b as HTMLInputElement).checked)).toBe(true);
    expect(sendButton()).toHaveTextContent("Send 3 notes to agent");
    fireEvent.click(all);
    expect(sendButton()).toBeNull();
    fireEvent.click(all);
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(sendButton()).toBeNull();
    r.unmount();
  });

  it("does not count a selected note the page no longer shows", () => {
    const r = render(() => <List notes={ids(3)} />);
    fireEvent.click(screen.getByRole("checkbox", { name: "Select all on this page" }));
    fireEvent.click(screen.getByRole("button", { name: "Drop first" }));
    expect(sendButton()).toHaveTextContent("Send 2 notes to agent");
    r.unmount();
  });

  it("shows the cap and refuses a send of more notes than it", () => {
    const r = render(() => <List notes={ids(MAX_NOTES + 1)} />);
    fireEvent.click(screen.getByRole("checkbox", { name: "Select all on this page" }));
    expect(screen.getByRole("alert")).toHaveTextContent(`One send takes at most ${MAX_NOTES} notes`);
    expect(sendButton()).toBeDisabled();
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Note 01JN0000" }));
    expect(sendButton()).toBeEnabled();
    expect(screen.queryByRole("alert")).toBeNull();
    r.unmount();
  });

  it("sends every selected note in one request, then clears the selection", async () => {
    const dispatch = vi.spyOn(api, "dispatch").mockResolvedValue({
      thread: { id: "01THREAD", agent: "clerk", title: "t", origin: "dispatch" },
    } as Awaited<ReturnType<typeof api.dispatch>>);
    vi.spyOn(api, "dispatched").mockResolvedValue({ threads: [] });
    const notes = ids(4);
    const r = render(() => <List notes={notes} />);
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Note 01JN0000" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Note 01JN0002" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Note 01JN0003" }));
    fireEvent.click(sendButton()!);
    expect(screen.getByRole("heading")).toHaveTextContent("Send 3 notes to an agent");
    fireEvent.input(screen.getByRole("textbox"), { target: { value: "File these" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Sent to Clerk.");
    expect(dispatch).toHaveBeenCalledTimes(1);
    expect(dispatch).toHaveBeenCalledWith("clerk", [notes[0], notes[2], notes[3]], "File these");
    // The dialog keeps what it sent; the list's selection is empty again.
    expect(screen.getByRole("heading")).toHaveTextContent("Send 3 notes to an agent");
    expect(screen.getAllByRole("checkbox", { name: /^Select Note/ }).some((b) => (b as HTMLInputElement).checked)).toBe(false);
    expect(sendButton()).toBeNull();
    fireEvent.click(screen.getByText("Close", { selector: "button" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    r.unmount();
  });
});

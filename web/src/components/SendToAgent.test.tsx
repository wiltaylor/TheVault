// Send to agent (TV-78): the dialog offers the enabled agents, needs an instruction, posts every
// note in one request, links the new thread, and shows what the server refuses.

import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { refreshAgents } from "../agents";
import { type AgentSummary, ApiError, api } from "../api";
import { SendToAgentButton, SendToAgentDialog } from "./SendToAgent";

const AGENTS: AgentSummary[] = [
  { name: "assistant", label: "Assistant", icon: null, built_in: true, enabled: true },
  { name: "clerk", label: "Clerk", icon: "pen", built_in: false, enabled: true },
  { name: "sleepy", label: "Sleepy", icon: null, built_in: false, enabled: false },
] as AgentSummary[];

const NOTE = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const OTHER = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC";

beforeEach(async () => {
  vi.spyOn(api, "agents").mockResolvedValue({ agents: AGENTS, tools: [] });
  await refreshAgents();
});

afterEach(() => vi.restoreAllMocks());

function thread(agent: string, id: string) {
  return { thread: { id, agent, title: "t", origin: "dispatch", pending: 0, state: "active" as const, notes: [NOTE] } };
}

describe("the Send to agent dialog", () => {
  it("offers only enabled agents and needs an instruction", async () => {
    const r = render(() => <SendToAgentDialog notes={[NOTE]} onClose={() => {}} />);
    const options = [...r.getByRole("combobox").querySelectorAll("option")].map((o) => o.textContent);
    expect(options).toEqual(["Assistant", "Clerk"]);
    const send = r.getByRole("button", { name: "Send" });
    expect(send).toBeDisabled();
    fireEvent.input(r.getByRole("textbox"), { target: { value: "   " } });
    expect(send).toBeDisabled();
    fireEvent.input(r.getByRole("textbox"), { target: { value: "Summarise it" } });
    expect(send).toBeEnabled();
    r.unmount();
  });

  it("sends every note in one request to the picked agent and links the thread", async () => {
    const dispatch = vi.spyOn(api, "dispatch").mockResolvedValue(thread("clerk", "01THREAD"));
    const onSent = vi.fn();
    const r = render(() => <SendToAgentDialog notes={[NOTE, OTHER]} onClose={() => {}} onSent={onSent} />);
    expect(r.getByRole("heading")).toHaveTextContent("Send 2 notes to an agent");
    fireEvent.change(r.getByRole("combobox"), { target: { value: "clerk" } });
    fireEvent.input(r.getByRole("textbox"), { target: { value: "  File these  " } });
    fireEvent.click(r.getByRole("button", { name: "Send" }));
    expect(await r.findByRole("status")).toHaveTextContent("Sent to Clerk.");
    expect(dispatch).toHaveBeenCalledTimes(1);
    expect(dispatch).toHaveBeenCalledWith("clerk", [NOTE, OTHER], "File these");
    expect(onSent).toHaveBeenCalledWith("clerk", "01THREAD");
    expect(r.getByRole("link", { name: "Open thread" })).toHaveAttribute("href", "/agents/clerk?thread=01THREAD");
    r.unmount();
  });

  it.each([
    [400, { error: "bad_request", message: "the agent may not read notes of type secret" }, "the agent may not read notes of type secret"],
    [403, { error: "forbidden", message: "needs write" }, "This device may not send notes to an agent: it needs a token that may write."],
    [429, { error: "too_many_runs", message: "clerk started 60 runs in the last hour: try again later" }, "clerk started 60 runs in the last hour: try again later"],
  ])("shows a %i in the dialog and keeps the instruction", async (status, body, shown) => {
    vi.spyOn(api, "dispatch").mockRejectedValue(new ApiError(status, body));
    const onClose = vi.fn();
    const r = render(() => <SendToAgentDialog notes={[NOTE]} onClose={onClose} />);
    fireEvent.input(r.getByRole("textbox"), { target: { value: "Do it" } });
    fireEvent.click(r.getByRole("button", { name: "Send" }));
    expect(await r.findByRole("alert")).toHaveTextContent(shown);
    expect(r.getByRole("textbox")).toHaveValue("Do it");
    expect(r.getByRole("button", { name: "Send" })).toBeEnabled();
    expect(onClose).not.toHaveBeenCalled();
    r.unmount();
  });

  it("closes on Escape", () => {
    const onClose = vi.fn();
    const r = render(() => <SendToAgentDialog notes={[NOTE]} onClose={onClose} />);
    fireEvent.keyDown(r.getByRole("dialog"), { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
    r.unmount();
  });
});

describe("the Send to agent button", () => {
  it("opens the dialog for its notes", async () => {
    const r = render(() => <SendToAgentButton notes={[NOTE]} />);
    expect(r.queryByRole("dialog")).toBeNull();
    fireEvent.click(r.getByRole("button", { name: "Send to agent" }));
    await waitFor(() => expect(r.getByRole("dialog")).toBeInTheDocument());
    expect(r.getByRole("heading")).toHaveTextContent("Send this note to an agent");
    r.unmount();
  });
});

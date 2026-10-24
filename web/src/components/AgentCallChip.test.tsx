// A call of an agent committed in a note shows as a chip (agents G19): the agent's icon and the
// call's text, the running spinner while its thread runs, "Sending…" until the server has seen
// it, and a link that opens the thread.

import { render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { agentCallHref, newUlid, parseAgentCall, resetCalls } from "../agentCalls";
import { refreshAgents } from "../agents";
import { bumpTurns } from "../ai/running";
import { type AgentSummary, ApiError, api } from "../api";
import { Inline } from "./Inline";

const CALL = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const AGENTS: AgentSummary[] = [{ name: "clerk", label: "Clerk", icon: "pen", built_in: false, enabled: true, mention: { types: null } }];

beforeEach(async () => {
  resetCalls();
  vi.spyOn(api, "agents").mockResolvedValue({ agents: AGENTS, tools: [] });
  await refreshAgents();
});

afterEach(() => vi.restoreAllMocks());

describe("agent call links", () => {
  it("read and write the agent and call id", () => {
    expect(parseAgentCall(agentCallHref("clerk", CALL))).toEqual({ agent: "clerk", call: CALL });
    expect(parseAgentCall("agent:clerk")).toBeNull();
    expect(parseAgentCall("https://example.com")).toBeNull();
    const id = newUlid(Date.UTC(2026, 0, 5));
    expect(id).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/);
    expect(id.slice(0, 10)).toBe(newUlid(Date.UTC(2026, 0, 5)).slice(0, 10));
    expect(newUlid()).not.toBe(newUlid());
  });
});

describe("the chip", () => {
  const text = `Roof: [@Clerk tidy this](agent:clerk?call=${CALL}) today`;

  it("shows the call running and opens its thread", async () => {
    const call = vi.spyOn(api, "agentCall").mockResolvedValue({ agent: "clerk", note: "01N", thread: "01THREAD", running: true });
    const r = render(() => <Inline text={text} />);
    const chip = await r.findByTestId("agent-call");
    expect(chip).toHaveTextContent("@Clerk tidy this");
    expect(await r.findByRole("img", { name: "Running" })).toBeInTheDocument();
    await waitFor(() => expect(chip).toHaveAttribute("href", "/agents/clerk?thread=01THREAD"));
    expect(call).toHaveBeenCalledWith(CALL);
    // The run ends: the spinner goes.
    call.mockResolvedValue({ agent: "clerk", note: "01N", thread: "01THREAD", running: false });
    bumpTurns();
    await waitFor(() => expect(r.queryByRole("img", { name: "Running" })).toBeNull());
    r.unmount();
  });

  it("says Sending… until the server has seen the call, and Not answered for a call it did not run", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const call = vi.spyOn(api, "agentCall").mockRejectedValue(new ApiError(404, { error: "not_found", message: "not seen" }));
      const r = render(() => <Inline text={text} />);
      expect(r.getByTestId("agent-call-sending")).toHaveTextContent("Sending…");
      expect(r.getByTestId("agent-call")).toHaveAttribute("href", "/agents/clerk");
      call.mockResolvedValue({ agent: "clerk", note: "01N", thread: null, running: false });
      await vi.advanceTimersByTimeAsync(3_000);
      expect(await r.findByText("Not answered")).toBeInTheDocument();
      expect(r.queryByTestId("agent-call-sending")).toBeNull();
      r.unmount();
    } finally {
      vi.useRealTimers();
    }
  });

  it("leaves other links alone", () => {
    const r = render(() => <Inline text="[@Clerk](/agents/clerk/chat?thread=01T) and [x](https://example.com)" />);
    expect(r.queryByTestId("agent-call")).toBeNull();
    expect(r.getByText("@Clerk").closest("a")).toHaveAttribute("href", "/agents/clerk/chat?thread=01T");
    r.unmount();
  });
});

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AgentCode } from "../api";

const createAgentCode = vi.fn();
const copyText = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, createAgentCode } };
});
vi.mock("../clipboard", () => ({ copyText }));

const { ConnectAgent } = await import("./Access");

const issued: AgentCode = {
  code: "K7QM-3XRT",
  expires: Date.now() + 10 * 60 * 1000,
  name: "agent-claude",
  scope: "write",
  server_url: "https://vault.example",
  command: "curl -fsSL https://vault.example/agent/install.sh | sh -s -- K7QM-3XRT",
};

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ConnectAgent", () => {
  it("defaults to write and warns only when admin is chosen", () => {
    render(() => <ConnectAgent />);
    const scope = screen.getByLabelText("Agent scope") as HTMLSelectElement;
    expect(scope.value).toBe("write");
    expect(screen.queryByText(/can change types, scripts and tasks/)).toBeNull();
    fireEvent.change(scope, { target: { value: "admin" } });
    expect(screen.getByText(/can change types, scripts and tasks and read secrets through scripts/)).toBeTruthy();
    fireEvent.change(scope, { target: { value: "read" } });
    expect(screen.queryByText(/can change types, scripts and tasks/)).toBeNull();
    expect(screen.getByText(/needs a shell/)).toBeTruthy();
  });

  it("shows the one-line command with a copy button and the expiry", async () => {
    createAgentCode.mockResolvedValue(issued);
    copyText.mockResolvedValue(true);
    render(() => <ConnectAgent />);
    const create = screen.getByRole("button", { name: "Create install command" }) as HTMLButtonElement;
    expect(create.disabled).toBe(true);
    fireEvent.input(screen.getByLabelText("Agent name"), { target: { value: " claude " } });
    fireEvent.click(create);

    const shown = (await screen.findByLabelText("Install command")) as HTMLInputElement;
    expect(createAgentCode).toHaveBeenCalledWith("claude", "write", window.location.origin);
    expect(shown.value).toBe(issued.command);
    expect(screen.getByText(/works once and expires in 10 minutes/)).toBeTruthy();
    expect(screen.getByText(/agent-claude/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(copyText).toHaveBeenCalledWith(issued.command));
    expect(await screen.findByText("Copied.")).toBeTruthy();
  });

  it("sends the chosen scope and shows a refusal", async () => {
    createAgentCode.mockRejectedValue(new Error("agent name must be plain"));
    render(() => <ConnectAgent />);
    fireEvent.input(screen.getByLabelText("Agent name"), { target: { value: "boss" } });
    fireEvent.change(screen.getByLabelText("Agent scope"), { target: { value: "admin" } });
    fireEvent.click(screen.getByRole("button", { name: "Create install command" }));
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(createAgentCode).toHaveBeenCalledWith("boss", "admin", window.location.origin);
    expect(screen.queryByLabelText("Install command")).toBeNull();
  });
});

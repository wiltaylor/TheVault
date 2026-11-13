import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AgentCode } from "../api";

const createAgentCode = vi.fn();
const copyText = vi.fn();
const tokens = vi.fn();
const createToken = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, createAgentCode, tokens, createToken } };
});
vi.mock("../clipboard", () => ({ copyText }));

const { ConnectAgent, Tokens } = await import("./Access");

it("shows stored user, device and agent kinds without inferring from names", async () => {
  tokens.mockResolvedValue(["user", "device", "agent"].map((kind, i) => ({
    name: `plain-${i}`, kind, scope: "write", created: 1, last_used: null,
  })));
  render(() => <Tokens />);
  const table = await screen.findByRole("table", { name: "Tokens" });
  await waitFor(() => expect(table.querySelectorAll("tbody tr").length).toBe(3));
  for (const [i, kind] of ["user", "device", "agent"].entries()) {
    expect(table.querySelector(`[data-token="plain-${i}"]`)?.textContent).toContain(kind);
  }
  expect(table.querySelector(".agent-badge")?.textContent).toBe("agent");
});

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
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

it("shows legacy, expired, stale and never-used tokens at the 90-day boundary", async () => {
  const now = Date.UTC(2026, 10, 13);
  vi.spyOn(Date, "now").mockReturnValue(now);
  const old = now - 90 * 86400000;
  tokens.mockResolvedValue([
    { name: "legacy", kind: "user", scope: "read", created: old, last_used: null, expires: null },
    { name: "expired", kind: "agent", scope: "write", created: old, last_used: old, expires: now },
    { name: "recent", kind: "device", scope: "write", created: old, last_used: old + 1, expires: now + 1 },
  ]);
  render(() => <Tokens />);
  const table = await screen.findByRole("table", { name: "Tokens" });
  expect(table.querySelector('[data-token="legacy"]')?.textContent).toContain("No expiry");
  expect(table.querySelector('[data-token="legacy"]')?.textContent).toContain("Never used in 90 days");
  expect(table.querySelector('[data-token="expired"]')?.textContent).toContain("Expired");
  expect(table.querySelector('[data-token="expired"]')?.textContent).toContain("Inactive for 90 days or more");
  expect(table.querySelector('[data-token="recent"]')?.textContent).not.toMatch(/Expired|Inactive|Never used/);
  expect(screen.getByText(/Agent tokens expire 90 days after/)).toBeTruthy();
});

it("sends optional expiry and shows the returned expiry without redisplaying listed secrets", async () => {
  tokens.mockResolvedValue([]);
  const date = "2099-01-02T12:00";
  const expires = new Date(date).getTime();
  createToken.mockResolvedValue({ name: "cli", kind: "user", scope: "read", expires, secret: "one-time" });
  render(() => <Tokens />);
  fireEvent.input(await screen.findByLabelText("Name"), { target: { value: "cli" } });
  fireEvent.input(screen.getByLabelText("Expiry (local time, optional)"), { target: { value: date } });
  fireEvent.click(screen.getByRole("button", { name: "Create token" }));
  await screen.findByLabelText("Token secret");
  expect(createToken).toHaveBeenCalledWith("cli", "read", expires);
  expect(screen.getByText(/^Expiry:/)?.textContent).not.toContain("No expiry");
  fireEvent.click(screen.getByRole("button", { name: "Done" }));
  expect(screen.queryByLabelText("Token secret")).toBeNull();
  fireEvent.input(screen.getByLabelText("Expiry (local time, optional)"), { target: { value: "" } });
  fireEvent.input(screen.getByLabelText("Name"), { target: { value: "cli2" } });
  await waitFor(() => expect((screen.getByRole("button", { name: "Create token" }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "Create token" }));
  await waitFor(() => expect(createToken).toHaveBeenLastCalledWith("cli2", "read", null));
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

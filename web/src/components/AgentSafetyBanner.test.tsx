import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "../api";
import { AgentSafetyBanner } from "./AgentSafetyBanner";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it("shows a held reason and releases only the reviewed fingerprint", async () => {
  const release = vi.spyOn(api, "releaseAgentSafety").mockResolvedValue({ agent_safety: { status: "released", reason: "user_released", fingerprint: "hash" } });
  render(() => <AgentSafetyBanner id="note" safety={{ status: "held", reason: "prompt_injection", fingerprint: "hash" }} />);
  expect(screen.getByText("Held from agents")).toBeInTheDocument();
  expect(screen.getByText(/Possible instructions aimed at an AI agent/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Release reviewed note to agents" }));
  await waitFor(() => expect(release).toHaveBeenCalledWith("note", "hash"));
  await waitFor(() => expect(screen.queryByText("Held from agents")).toBeNull());
});

it("keeps a stale or denied release held and shows the error", async () => {
  vi.spyOn(api, "releaseAgentSafety").mockRejectedValue(new Error("The note changed"));
  render(() => <AgentSafetyBanner id="note" safety={{ status: "held", reason: "sensitive_credentials", fingerprint: "old" }} />);
  fireEvent.click(screen.getByRole("button", { name: "Release reviewed note to agents" }));
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("The note changed"));
  expect(screen.getByText("Held from agents")).toBeInTheDocument();
});

it("does not show a hold for an allowed note", () => {
  render(() => <AgentSafetyBanner id="note" safety={{ status: "allowed", reason: "marketing", fingerprint: "hash" }} />);
  expect(screen.queryByText("Held from agents")).toBeNull();
});

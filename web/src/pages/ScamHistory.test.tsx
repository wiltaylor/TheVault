import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

const preview = vi.fn();
const approve = vi.fn();
const progress = vi.fn();
vi.mock("../api", async (original) => ({
  ...await original<typeof import("../api")>(),
  api: { scamHistoryPreview: preview, scamHistoryApprove: approve, scamHistoryProgress: progress },
}));
const { ScamHistory } = await import("./ScamHistory");
const shown = { fingerprint: "consent-digest", days: 30, until_ms: 0, count: 3, account: "Fake router", model: "jev", decisions_per_item: 2, unit_price: 0.25, estimated_usd: 1.5 };
beforeEach(() => {
  vi.resetAllMocks(); preview.mockResolvedValue(shown);
  approve.mockResolvedValue({ status: "running", count: 3, attempts: [] });
  progress.mockResolvedValue({ status: "complete", count: 3, attempts: [{ id: "fixture", outcome: "provider_failed" }] });
});
afterEach(cleanup);

it("is collapsed with no provider/action calls and requires explicit valid price and window", async () => {
  render(() => <ScamHistory />);
  expect(screen.getByLabelText("Historical scam run").hasAttribute("open")).toBe(false);
  expect(preview).not.toHaveBeenCalled(); expect(progress).not.toHaveBeenCalled();
  const button = screen.getByRole("button", { name: "Preview historical scam run" });
  expect(button).toBeDisabled();
  fireEvent.input(screen.getByLabelText("Estimated USD per decision"), { target: { value: "0" } });
  expect(button).not.toBeDisabled();
  for (const days of ["0", "91"]) {
    fireEvent.input(screen.getByLabelText("Historical days"), { target: { value: days } }); expect(button).toBeDisabled();
  }
});

it("shows exact estimate and fingerprint, approves only that preview, and shows durable failures", async () => {
  render(() => <ScamHistory />);
  screen.getByLabelText("Historical scam run").setAttribute("open", "");
  fireEvent.input(screen.getByLabelText("Estimated USD per decision"), { target: { value: "0.25" } });
  fireEvent.click(screen.getByText("Preview historical scam run"));
  await screen.findByText("consent-digest");
  expect(preview).toHaveBeenCalledWith(30, 0.25);
  expect(screen.getByText(/Estimated total:/).textContent).toContain("$1.5 USD");
  fireEvent.click(screen.getByText("Approve historical scam run"));
  await waitFor(() => expect(approve).toHaveBeenCalledWith("consent-digest", 30, 0.25));
  await waitFor(() => expect(screen.getByText("Refresh scam run progress")).not.toBeDisabled());
  fireEvent.click(screen.getByText("Refresh scam run progress"));
  await screen.findByText("fixture: provider_failed");
});

it("invalidates approval when either input changes or the server refuses a stale preview", async () => {
  render(() => <ScamHistory />);
  screen.getByLabelText("Historical scam run").setAttribute("open", "");
  fireEvent.input(screen.getByLabelText("Estimated USD per decision"), { target: { value: "0.25" } });
  fireEvent.click(screen.getByText("Preview historical scam run")); await screen.findByText("consent-digest");
  await waitFor(() => expect(screen.getByLabelText("Historical days")).not.toBeDisabled());
  fireEvent.input(screen.getByLabelText("Historical days"), { target: { value: "31" } });
  expect(screen.queryByText("Approve historical scam run")).toBeNull();
  fireEvent.click(screen.getByText("Preview historical scam run")); await screen.findByText("consent-digest");
  await waitFor(() => expect(screen.getByLabelText("Estimated USD per decision")).not.toBeDisabled());
  fireEvent.input(screen.getByLabelText("Estimated USD per decision"), { target: { value: "0.5" } });
  expect(screen.queryByText("Approve historical scam run")).toBeNull();
  fireEvent.click(screen.getByText("Preview historical scam run")); await screen.findByText("consent-digest");
  await waitFor(() => expect(screen.getByText("Approve historical scam run")).not.toBeDisabled());
  approve.mockRejectedValue(new Error("stale_preview"));
  fireEvent.click(screen.getByText("Approve historical scam run")); await screen.findByRole("alert");
  expect(screen.queryByText("Approve historical scam run")).toBeNull();
});

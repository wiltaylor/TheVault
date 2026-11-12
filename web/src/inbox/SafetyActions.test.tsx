import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "../api";
import { SafetyActions } from "./SafetyActions";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });
const estimate = { preview: "p", count: 3, account: "fixture", model: "jev", decisions_per_note: 2, unit_price: 0.01, estimated_usd: 0.06, disclaimer: "Owner-entered estimate, not a billing guarantee." };

it("requires explicit price and approval after showing the calculation", async () => {
  const preview = vi.spyOn(api, "safetyBackfillPreview").mockResolvedValue(estimate);
  const approve = vi.spyOn(api, "safetyBackfillApprove").mockResolvedValue({ status: "running", count: 3, attempted: 0 });
  render(() => <SafetyActions selected={[]} onReleased={() => {}} />);
  const button = screen.getByText("Preview safety pass") as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  fireEvent.input(screen.getByLabelText("Owner-entered estimated USD per classifier decision"), { target: { value: "0.01" } });
  fireEvent.click(button);
  await screen.findByText("3 notes × 2 decisions per note × $0.01 = estimated $0.06 USD");
  expect(preview).toHaveBeenCalledWith(0.01);
  expect(approve).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Approve one-time safety pass"));
  await waitFor(() => expect(approve).toHaveBeenCalledWith("p"));
  await screen.findByText(/Safety pass: running/);
});

it("accepts explicit zero and invalidates displayed approval on price edits", async () => {
  vi.spyOn(api, "safetyBackfillPreview").mockResolvedValue({ ...estimate, unit_price: 0, estimated_usd: 0 });
  render(() => <SafetyActions selected={[]} onReleased={() => {}} />);
  const input = screen.getByLabelText("Owner-entered estimated USD per classifier decision");
  fireEvent.input(input, { target: { value: "0" } });
  fireEvent.click(screen.getByText("Preview safety pass"));
  await screen.findByText("Approve one-time safety pass");
  fireEvent.input(input, { target: { value: "2" } });
  expect(screen.queryByText("Approve one-time safety pass")).toBeNull();
});

it("previews release filters and reports stale approval without retry", async () => {
  const preview = vi.spyOn(api, "safetyReleasePreview").mockResolvedValue({ preview: "r", count: 4 });
  const approve = vi.spyOn(api, "safetyReleaseApprove").mockRejectedValue(new Error("Preview changed"));
  render(() => <SafetyActions selected={[]} onReleased={() => {}} />);
  fireEvent.change(screen.getByLabelText("Release type"), { target: { value: "email" } });
  fireEvent.input(screen.getByLabelText("Release reason"), { target: { value: "scam" } });
  fireEvent.input(screen.getByLabelText("Created since"), { target: { value: "2026-01-01" } });
  fireEvent.click(screen.getByText("Preview filtered release"));
  await screen.findByText("Release 4 held notes? Matching agent triggers may run.");
  expect(preview).toHaveBeenCalledWith({ type: "email", reason: "scam", since: "2026-01-01T00:00:00.000Z", until: undefined });
  expect(approve).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Confirm release"));
  await screen.findByText("Preview changed");
  expect(approve).toHaveBeenCalledTimes(1);
  expect(screen.queryByText("Confirm release")).toBeNull();
});

it("shows progress, failed count and holds by reason on refresh", async () => {
  vi.spyOn(api, "safetyBackfillProgress").mockResolvedValue({ status: "complete", count: 3, attempted: 3, allowed: 1, failed: 1, held: { scam: 1 } });
  render(() => <SafetyActions selected={[]} onReleased={() => {}} />);
  fireEvent.click(screen.getByText("Refresh safety progress"));
  await screen.findByText("Safety pass: complete. Attempted 3 / 3. Allowed 1. Failed 1.");
  expect(screen.getByText("Held: scam: 1")).toBeTruthy();
});

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "../api";
import { MailTriage } from "./MailTriage";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });
it("counts before confirmation, consumes the preview and labels recommendations read-only", async () => {
  const preview = vi.spyOn(api, "mailTriagePreview").mockResolvedValue({ count: 80, selected: 20, preview: "p" });
  const confirm = vi.spyOn(api, "mailTriageConfirm").mockResolvedValue({ queued: true });
  render(() => <MailTriage />);
  fireEvent.input(screen.getByLabelText("Agent name"), { target: { value: "owl" } });
  fireEvent.click(screen.getByText("Preview recommendation batch"));
  const button = await screen.findByText("Queue 20 recommendations as one summary");
  expect(preview).toHaveBeenCalledWith("owl", 20);
  expect(confirm).not.toHaveBeenCalled();
  expect(screen.getByText(/No Apply action/)).toBeTruthy();
  fireEvent.click(button);
  await waitFor(() => expect(confirm).toHaveBeenCalledWith("owl", "p"));
  await screen.findByText(/Queued. Read the recommendation/);
  expect(screen.queryByText("Queue 20 recommendations as one summary")).toBeNull();
});
it("invalidates approval when the batch or agent changes and shows errors", async () => {
  vi.spyOn(api, "mailTriagePreview").mockResolvedValueOnce({ count: 20, selected: 20, preview: "p" }).mockRejectedValueOnce(new Error("At least 20 eligible messages"));
  render(() => <MailTriage />);
  fireEvent.input(screen.getByLabelText("Agent name"), { target: { value: "owl" } });
  fireEvent.click(screen.getByText("Preview recommendation batch"));
  await screen.findByText("Queue 20 recommendations as one summary");
  fireEvent.input(screen.getByLabelText("Batch size"), { target: { value: "50" } });
  expect(screen.queryByText("Queue 20 recommendations as one summary")).toBeNull();
  fireEvent.click(screen.getByText("Preview recommendation batch"));
  await screen.findByText("At least 20 eligible messages");
});

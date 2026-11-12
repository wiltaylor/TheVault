import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, expect, it, vi } from "vitest";
import { api, type SafetyEvidence } from "../api";
import { AttachmentEvidence } from "./AttachmentEvidence";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });
const evidence = (current: boolean): SafetyEvidence => ({
  envelope: { envelope_version: 1, attachments: [
    { id: "a", filename: "invoice.csv", status: "ready", text: "<script>not markup</script>\n17" },
    { id: "b", status: "pending" }, { id: "c", status: "failed" }, { id: "d", status: "unsupported" },
  ] }, fingerprint: "hash", decision_current: current, decision_status: "allowed", bytes: 1024, limit: 131072,
  estimated_input_tokens: 256, omissions: [{ id: "html", reason: "HTML stays withheld" }],
});

it("reviews current exact text, byte cap, omissions and honest estimates on demand", async () => {
  const read = vi.spyOn(api, "safetyEvidence").mockResolvedValue(evidence(true));
  const view = render(() => <AttachmentEvidence id="note" />);
  expect(read).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Review attachment evidence"));
  expect(await screen.findByText("Current decision matches this envelope.")).toBeInTheDocument();
  expect(screen.getByText(/1024 \/ 131072 bytes/)).toBeInTheDocument();
  expect(screen.getByText(/256 tokens/)).toHaveTextContent("Prices are unknown. Actual usage is not recorded yet");
  expect(screen.getByText(/Text omitted: pending/)).toBeInTheDocument();
  expect(screen.getByText(/Text omitted: failed/)).toBeInTheDocument();
  expect(screen.getByText(/Text omitted: unsupported/)).toBeInTheDocument();
  expect(screen.getByText("html: HTML stays withheld")).toBeInTheDocument();
  expect(view.container.querySelector("script")).toBeNull();
  expect(view.container.textContent).toContain("<script>not markup</script>\n17");
});

it("never labels stale content as historical Jev evidence and explains over-cap denial", async () => {
  vi.spyOn(api, "safetyEvidence").mockResolvedValue({ ...evidence(false), bytes: 140000 });
  render(() => <AttachmentEvidence id="note" />);
  fireEvent.click(screen.getByText("Review attachment evidence"));
  expect(await screen.findByText(/Stale or missing envelope decision/)).toBeInTheDocument();
  expect(screen.getByText(/Over cap: no envelope is sent/)).toBeInTheDocument();
  expect(screen.queryByText("Current decision matches this envelope.")).toBeNull();
});

it("shows access errors without exposing evidence", async () => {
  vi.spyOn(api, "safetyEvidence").mockRejectedValue(new Error("Owner access required"));
  render(() => <AttachmentEvidence id="note" />);
  fireEvent.click(screen.getByText("Review attachment evidence"));
  expect(await screen.findByRole("alert")).toHaveTextContent("Owner access required");
  expect(screen.queryByText(/Envelope v/)).toBeNull();
});

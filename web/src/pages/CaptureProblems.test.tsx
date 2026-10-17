import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CaptureProblem } from "../api";

const captureProblems = vi.fn();
const retryCaptureProblem = vi.fn();
const dismissCaptureProblem = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, captureProblems, retryCaptureProblem, dismissCaptureProblem } };
});

const { CaptureProblems, CaptureProblemsBadge, eventLabel, problemStatus } = await import("./CaptureProblems");

const failed: CaptureProblem = {
  id: 2,
  event: "sms_received",
  at: 1_760_000_100_000,
  status: "failed",
  attempts: 3,
  message: "the message note could not be written",
  who: "Ada Lovelace",
};
const skipped: CaptureProblem = {
  id: 1,
  event: "sms_received",
  at: 1_760_000_000_000,
  status: "skipped",
  attempts: 0,
  message: "the body matches the skip pattern",
  who: "+61400000009",
};

function mount() {
  return render(() => (
    <div>
      <h2>
        This device <CaptureProblemsBadge />
      </h2>
      <CaptureProblems />
    </div>
  ));
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("CaptureProblems", () => {
  it("lists failed and skipped events newest first with who, when, the message and the attempts, and badges the failed count", async () => {
    captureProblems.mockResolvedValue({ device: true, failed: 1, events: [failed, skipped] });
    const m = mount();
    await waitFor(() => expect(m.container.querySelectorAll(".capture-problem")).toHaveLength(2));
    const [first, second] = [...m.container.querySelectorAll(".capture-problem")] as [Element, Element];
    expect(first).toHaveAttribute("data-status", "failed");
    expect(first.textContent).toContain("SMS received");
    expect(first.textContent).toContain("from Ada Lovelace");
    expect(first.textContent).toContain("Failed after 3 tries");
    expect(first.textContent).toContain("the message note could not be written");
    expect(second.textContent).toContain("Skipped");
    // Only a failed event has Retry and Dismiss.
    expect(m.getAllByRole("button", { name: "Retry" })).toHaveLength(1);
    expect(m.getAllByRole("button", { name: "Dismiss" })).toHaveLength(1);
    expect(m.getByRole("status", { name: "1 capture problem" })).toHaveTextContent("1");
  });

  it("retries a failed event and shows it saved, then reloads the list", async () => {
    captureProblems.mockResolvedValueOnce({ device: true, failed: 1, events: [failed, skipped] });
    retryCaptureProblem.mockResolvedValue({ ...failed, status: "done", attempts: 1, message: null, failed: 0 });
    captureProblems.mockResolvedValueOnce({ device: true, failed: 0, events: [skipped] });
    const m = mount();
    await waitFor(() => m.getByRole("button", { name: "Retry" }));
    fireEvent.click(m.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(m.getByText("SMS received saved.")).toBeInTheDocument());
    expect(retryCaptureProblem).toHaveBeenCalledWith(2);
    await waitFor(() => expect(m.container.querySelectorAll(".capture-problem")).toHaveLength(1));
    expect(m.queryByRole("status", { name: /capture problem/ })).toBeNull();
  });

  it("dismisses a failed event, which leaves the list", async () => {
    captureProblems.mockResolvedValueOnce({ device: true, failed: 1, events: [failed] });
    dismissCaptureProblem.mockResolvedValue({ id: 2, status: "dismissed", failed: 0 });
    captureProblems.mockResolvedValueOnce({ device: true, failed: 0, events: [] });
    const m = mount();
    await waitFor(() => m.getByRole("button", { name: "Dismiss" }));
    fireEvent.click(m.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(m.getByText("No capture problems.")).toBeInTheDocument());
    expect(dismissCaptureProblem).toHaveBeenCalledWith(2);
  });
});

describe("words", () => {
  it("names device events and how a problem stands", () => {
    expect(eventLabel("call_ended")).toBe("Call");
    expect(eventLabel("something_new")).toBe("something_new");
    expect(problemStatus({ ...failed, attempts: 1 })).toBe("Failed after 1 try");
    expect(problemStatus(skipped)).toBe("Skipped");
  });
});

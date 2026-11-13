import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ agentThread: vi.fn(), aiApplyKindPreview: vi.fn(), aiApplyKindConfirm: vi.fn() }));
vi.mock("../api", () => ({ api: mocks }));
vi.mock("../pages/TypeEditor", () => ({ errorText: (e: unknown) => String(e) }));
import { ApplyKind } from "./ApplyKind";
afterEach(cleanup);
beforeEach(() => {
  vi.resetAllMocks();
  mocks.agentThread.mockResolvedValue({ proposals: [
    { event: "proposal", status: "pending", data: { id: "p1", op: "create", type: "memo" } },
    { event: "proposal", status: "applied", data: { id: "old" } },
  ] });
  mocks.aiApplyKindPreview.mockResolvedValue({ preview: "snapshot", count: 2, kind: ["proposal", "create", "memo"],
    items: [{ id: "p1", summary: "First", data: {} }, { id: "p2", summary: "Second", data: {} }] });
  mocks.aiApplyKindConfirm.mockResolvedValue({ results: [{ id: "p1", ok: true }, { id: "p2", ok: false, error: { message: "Stale" } }] });
});
async function preview() {
  const m = render(() => <ApplyKind agent="reader" thread="thread1" />);
  fireEvent.click(m.getByText("Review by kind"));
  fireEvent.click(await m.findByText(/Preview this kind:/));
  await m.findByText("Confirm Apply 2 proposals");
  return m;
}
it("previews counted stable IDs without writing, then requires explicit confirmation", async () => {
  const m = await preview();
  expect(mocks.aiApplyKindPreview).toHaveBeenCalledWith("thread1", "p1");
  expect(mocks.aiApplyKindConfirm).not.toHaveBeenCalled();
  expect(m.getByText("p2")).toBeTruthy();
  fireEvent.click(m.getByText("Confirm Apply 2 proposals"));
  await m.findByText("p2: Stale");
  fireEvent.click(m.getByText("Retry unsuccessful proposals"));
  await waitFor(() => expect(mocks.aiApplyKindConfirm).toHaveBeenCalledTimes(2));
  expect(mocks.aiApplyKindConfirm.mock.calls).toEqual([["snapshot"], ["snapshot"]]);
});
it("closing preview never writes", async () => {
  const m = await preview();
  fireEvent.click(m.getByText("Close preview"));
  expect(m.queryByText("Confirm Apply 2 proposals")).toBeNull();
  expect(mocks.aiApplyKindConfirm).not.toHaveBeenCalled();
});
it("reports expired or failed confirmation without claiming success", async () => {
  const m = await preview();
  mocks.aiApplyKindConfirm.mockRejectedValue(new Error("Preview expired"));
  fireEvent.click(m.getByText("Confirm Apply 2 proposals"));
  expect(await m.findByRole("alert")).toHaveTextContent("Preview expired");
  expect(m.queryByText("p1: Applied")).toBeNull();
});

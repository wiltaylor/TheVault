import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api, type HeldNote } from "../api";
import { HeldInbox, loadHeld } from "./HeldInbox";

const held = (id: string): HeldNote => ({ id, type: "email", created: null, agent_safety: { status: "held", reason: "scam", fingerprint: id } });
beforeEach(() => {
  vi.spyOn(api, "note").mockImplementation(async (id) => ({ id, type: "email", version: "1", wcl: "", agent_safety: held(id).agent_safety, note: { fields: { title: `Subject ${id}`, from_raw: "<script>sender</script>", scam_probability: 0.9 }, blocks: [] } }));
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it("renders held metadata as text without agent controls and releases reviewed fingerprints", async () => {
  const release = vi.spyOn(api, "releaseAgentSafety").mockResolvedValue({ agent_safety: held("a").agent_safety });
  const done = vi.fn();
  render(() => <HeldInbox notes={[held("a")]} onReleased={done} />);
  await screen.findByText("Subject a");
  expect(screen.getByText("Scam probability: 90%")).toBeTruthy();
  expect(screen.getByText("<script>sender</script>")).toBeTruthy();
  expect(document.querySelector("script")).toBeNull();
  expect(screen.queryByText("Send to agent")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Release" }));
  await waitFor(() => expect(release).toHaveBeenCalledWith("a", "a"));
  expect(done).toHaveBeenCalledWith("a");
});

it("confirms server bulk count and keeps failures after partial success", async () => {
  const preview = vi.spyOn(api, "safetyReleasePreview").mockResolvedValue({ preview: "review", count: 2 });
  const release = vi.spyOn(api, "safetyReleaseApprove").mockResolvedValue({ released: 1, failed: 1, released_ids: ["a"] });
  render(() => {
    const [notes, setNotes] = createSignal([held("a"), held("b")]);
    return <HeldInbox notes={notes()} onReleased={(id) => setNotes((ns) => ns.filter((n) => n.id !== id))} />;
  });
  await screen.findByText("Subject b");
  fireEvent.click(screen.getByLabelText("Select Subject a"));
  fireEvent.click(screen.getByLabelText("Select Subject b"));
  fireEvent.click(screen.getByText("Release selected"));
  expect(await screen.findByText("Release 2 held notes? Matching agent triggers may run.")).toBeTruthy();
  expect(preview).toHaveBeenCalledWith({ ids: ["a", "b"] });
  expect(release).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Confirm release"));
  await screen.findByText("Released 1. Failed 1.");
  expect(screen.queryByText("Subject a")).toBeNull();
  expect(screen.getByText("Subject b")).toBeTruthy();
});

it("does not release changed fingerprints", async () => {
  vi.mocked(api.note).mockResolvedValue({ id: "a", type: "email", version: "2", wcl: "", agent_safety: { ...held("a").agent_safety, fingerprint: "new" }, note: { fields: { title: "Changed" }, blocks: [] } });
  render(() => <HeldInbox notes={[held("a")]} onReleased={() => {}} />);
  await screen.findByText("Changed");
  expect((screen.getByText("Release") as HTMLButtonElement).disabled).toBe(true);
});

it("loads every metadata page", async () => {
  vi.spyOn(api, "heldAgentNotes").mockResolvedValueOnce({ notes: [held("a")], total: 2 }).mockResolvedValueOnce({ notes: [held("b")], total: 2 });
  expect((await loadHeld()).map((n) => n.id)).toEqual(["a", "b"]);
  expect(api.heldAgentNotes).toHaveBeenLastCalledWith(1);
});

it("reports load failures and disables individual release", async () => {
  vi.mocked(api.note).mockRejectedValue(new Error("Cannot load review"));
  render(() => <HeldInbox notes={[held("a")]} onReleased={() => {}} />);
  expect((await screen.findByRole("alert")).textContent).toContain("Cannot load review");
  expect((screen.getByText("Release") as HTMLButtonElement).disabled).toBe(true);
});

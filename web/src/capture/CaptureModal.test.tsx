import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const createNote = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, createNote } };
});
const upload = vi.fn();
vi.mock("../blocks/MediaBlocks", () => ({ upload }));
const closeCapture = vi.fn();
vi.mock("./store", () => ({ closeCapture }));
const showToast = vi.fn();
vi.mock("../components/Toasts", () => ({ showToast }));

const { ApiError } = await import("../api");
const { CaptureModal } = await import("./CaptureModal");

function mount() {
  const r = render(() => <CaptureModal />);
  const area = () => r.getByRole("textbox", { name: "Capture text" }) as HTMLTextAreaElement;
  const save = () => r.getByRole("button", { name: "Save" }) as HTMLButtonElement;
  const type = (text: string) => {
    area().value = text;
    fireEvent.input(area());
  };
  return { ...r, area, save, type };
}

beforeEach(() => {
  URL.createObjectURL = vi.fn(() => "blob:x");
  URL.revokeObjectURL = vi.fn();
});

afterEach(() => {
  // Each dialog keeps focus in itself, so two left mounted would pull focus back and forth.
  cleanup();
  vi.clearAllMocks();
});

describe("CaptureModal", () => {
  it("is a modal dialog with the text box focused and Save disabled while empty", () => {
    const m = mount();
    const dialog = m.getByRole("dialog", { name: "Capture" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(m.area()).toHaveFocus();
    expect(m.save()).toBeDisabled();
    m.type("   ");
    expect(m.save()).toBeDisabled();
    m.type("Buy milk");
    expect(m.save()).toBeEnabled();
  });

  it("saves a fleeting note on Ctrl+Enter, then closes and offers to open it", async () => {
    createNote.mockResolvedValue({ id: "N1", type: "fleeting", version: "v1", wcl: "", note: { fields: { title: "Buy milk" }, blocks: [] } });
    const m = mount();
    m.type("Buy milk\nand eggs");
    fireEvent.keyDown(m.area(), { key: "Enter", ctrlKey: true });
    await waitFor(() => expect(closeCapture).toHaveBeenCalled());
    expect(createNote).toHaveBeenCalledWith("fleeting", {
      fields: { title: "Buy milk", processed: false },
      blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: "Buy milk\nand eggs" } }] }],
    });
    expect(showToast).toHaveBeenCalledWith("Saved to inbox", { label: "Open", href: "/notes/N1" });
  });

  it("shows a failed save in the dialog and keeps what was typed", async () => {
    createNote.mockRejectedValue(new ApiError(500, { message: "disk full" }));
    const m = mount();
    m.type("Keep me");
    fireEvent.click(m.save());
    expect(await m.findByRole("alert")).toHaveTextContent("disk full");
    expect(m.area().value).toBe("Keep me");
    expect(m.save()).toBeEnabled();
    expect(closeCapture).not.toHaveBeenCalled();
  });

  it("uploads pasted images and saves an image-only capture", async () => {
    upload.mockResolvedValue({ id: "A1", filename: "shot.png", size: 3, path: "assets/A1/shot.png" });
    createNote.mockResolvedValue({ id: "N2", type: "fleeting", version: "v1", wcl: "", note: { fields: { title: "Image" }, blocks: [] } });
    const m = mount();
    const file = new File(["png"], "shot.png", { type: "image/png" });
    fireEvent.paste(m.getByRole("dialog"), { clipboardData: { items: [{ kind: "file", getAsFile: () => file }] } });
    expect(m.getByRole("list", { name: "Attached images" }).querySelectorAll("img").length).toBe(1);
    expect(m.save()).toBeEnabled();
    fireEvent.click(m.save());
    await waitFor(() => expect(closeCapture).toHaveBeenCalled());
    expect(upload).toHaveBeenCalledWith(file);
    const doc = createNote.mock.calls[0]![1];
    expect(doc.fields.title).toMatch(/^Image \d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
  });

  it("removes an attached image with its ×", () => {
    const m = mount();
    const file = new File(["png"], "a.png", { type: "image/png" });
    fireEvent.paste(m.getByRole("dialog"), { clipboardData: { items: [{ kind: "file", getAsFile: () => file }] } });
    fireEvent.click(m.getByRole("button", { name: "Remove image 1" }));
    expect(m.queryByRole("list", { name: "Attached images" })).toBeNull();
    expect(m.save()).toBeDisabled();
  });

  it("closes on Escape when empty, and asks first when there is something to lose", () => {
    const m = mount();
    fireEvent.keyDown(m.area(), { key: "Escape" });
    expect(closeCapture).toHaveBeenCalledTimes(1);
    m.type("half a thought");
    fireEvent.keyDown(m.area(), { key: "Escape" });
    expect(closeCapture).toHaveBeenCalledTimes(1);
    expect(m.getByRole("group", { name: "Discard this capture?" })).toBeInTheDocument();
    fireEvent.click(m.getByRole("button", { name: "Keep editing" }));
    expect(m.area().value).toBe("half a thought");
    fireEvent.keyDown(m.area(), { key: "Escape" });
    fireEvent.click(m.getByRole("button", { name: "Discard" }));
    expect(closeCapture).toHaveBeenCalledTimes(2);
  });
});

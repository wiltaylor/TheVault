import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const createNote = vi.fn();
const agentsList = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, createNote, agents: agentsList } };
});
const appInfo = vi.fn(async () => null as { platform?: string } | null);
vi.mock("../inApp", () => ({ appInfo }));
const upload = vi.fn();
vi.mock("../blocks/MediaBlocks", () => ({ upload }));
const closeCapture = vi.fn();
vi.mock("./store", async (orig) => ({ ...(await orig<typeof import("./store")>()), closeCapture }));
const showToast = vi.fn();
vi.mock("../components/Toasts", () => ({ showToast }));

const { ApiError } = await import("../api");
const { CaptureModal } = await import("./CaptureModal");
const { refreshAgents } = await import("../agents");
const { SEND_TO_KEY } = await import("./store");

const ULID = /[0-9A-HJKMNP-TV-Z]{26}/;

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

/** Pretend to be a phone (a coarse pointer), or not. */
function setPhone(phone: boolean) {
  window.matchMedia = vi.fn((q: string) => ({ matches: phone && q.includes("coarse"), media: q })) as unknown as typeof window.matchMedia;
}

/** Give the browser a camera, or none. */
function setCamera(stream: MediaStream | null) {
  Object.defineProperty(navigator, "mediaDevices", {
    configurable: true,
    value: stream
      ? {
          enumerateDevices: vi.fn(async () => [{ kind: "videoinput", deviceId: "c1", label: "", groupId: "" }]),
          getUserMedia: vi.fn(async () => stream),
        }
      : undefined,
  });
}

const saved = (id: string, title = "x") => ({ id, type: "fleeting", version: "v1", wcl: "", note: { fields: { title }, blocks: [] } });

beforeEach(async () => {
  URL.createObjectURL = vi.fn(() => "blob:x");
  URL.revokeObjectURL = vi.fn();
  localStorage.clear();
  setPhone(false);
  setCamera(null);
  agentsList.mockResolvedValue({
    agents: [
      { name: "assistant", label: "Assistant", enabled: true, mention: { types: null } },
      { name: "clerk", label: "Clerk", enabled: true, mention: { types: ["fleeting"] } },
      { name: "off", label: "Off", enabled: false, mention: null },
      { name: "tasks_only", label: "Tasks only", enabled: true, mention: { types: ["task"] } },
    ],
    tools: [],
  });
  await refreshAgents();
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
    createNote.mockResolvedValue(saved("N1", "Buy milk"));
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
    createNote.mockResolvedValue(saved("N2", "Image"));
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

describe("Camera", () => {
  it("on a phone is a file input that asks for the camera, and the photo is attached", async () => {
    setPhone(true);
    const m = mount();
    const button = await m.findByRole("button", { name: "Camera" });
    const input = m.getByTestId("capture-camera-input") as HTMLInputElement;
    expect(input).toHaveAttribute("type", "file");
    expect(input).toHaveAttribute("accept", "image/*");
    expect(input).toHaveAttribute("capture", "environment");
    const click = vi.spyOn(input, "click");
    fireEvent.click(button);
    expect(click).toHaveBeenCalled();
    const photo = new File(["jpg"], "IMG_1.jpg", { type: "image/jpeg" });
    fireEvent.change(input, { target: { files: [photo] } });
    expect(m.getByRole("list", { name: "Attached images" }).querySelectorAll("img").length).toBe(1);
  });

  it("in the Android app is the file input too", async () => {
    appInfo.mockResolvedValueOnce({ platform: "android" });
    const m = mount();
    await m.findByRole("button", { name: "Camera" });
    expect(m.getByTestId("capture-camera-input")).toHaveAttribute("capture", "environment");
  });

  it("on a desktop with a camera opens a preview: Take photo, Retake, Use photo", async () => {
    const stop = vi.fn();
    const track = { stop, getSettings: () => ({ width: 4000, height: 3000 }) };
    const stream = { getTracks: () => [track], getVideoTracks: () => [track] } as unknown as MediaStream;
    setCamera(stream);
    const drawImage = vi.fn();
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ drawImage } as unknown as CanvasRenderingContext2D);
    const toBlob = vi
      .spyOn(HTMLCanvasElement.prototype, "toBlob")
      .mockImplementation((cb: BlobCallback, type?: string) => cb(new Blob(["jpg"], { type: type ?? "" })));
    const m = mount();
    fireEvent.click(await m.findByRole("button", { name: "Camera" }));
    expect(m.queryByTestId("capture-camera-input")).toBeNull();
    const camera = m.getByRole("group", { name: "Camera" });
    expect(navigator.mediaDevices.getUserMedia).toHaveBeenCalled();
    const take = m.getByRole("button", { name: "Take photo" });
    await waitFor(() => expect(take).toBeEnabled());
    expect((m.getByLabelText("Camera preview") as HTMLVideoElement).srcObject).toBe(stream);
    fireEvent.click(take);
    // The camera's 4000x3000 frame, capped at a 2560 px long edge.
    expect(drawImage).toHaveBeenCalledWith(expect.anything(), 0, 0, 2560, 1920);
    expect(toBlob).toHaveBeenCalledWith(expect.any(Function), "image/jpeg", expect.any(Number));
    expect(m.getByRole("img", { name: "Photo taken" })).toBeInTheDocument();
    fireEvent.click(m.getByRole("button", { name: "Retake" }));
    expect(m.queryByRole("img", { name: "Photo taken" })).toBeNull();
    fireEvent.click(m.getByRole("button", { name: "Take photo" }));
    fireEvent.click(m.getByRole("button", { name: "Use photo" }));
    expect(camera).not.toBeInTheDocument();
    expect(stop).toHaveBeenCalled();
    const thumbs = m.getByRole("list", { name: "Attached images" }).querySelectorAll("img");
    expect(thumbs.length).toBe(1);
    expect(thumbs[0]!.alt).toMatch(/^photo-\d{8}-\d{6}\.jpg$/);
    getContext.mockRestore();
    toBlob.mockRestore();
  });

  it("is hidden on a desktop without a camera", async () => {
    setCamera(null);
    const m = mount();
    await waitFor(() => expect(appInfo).toHaveBeenCalled());
    await Promise.resolve();
    expect(m.queryByRole("button", { name: "Camera" })).toBeNull();
    expect(m.getByRole("button", { name: "Add image" })).toBeInTheDocument();
  });
});

describe("Pen", () => {
  function draw(svg: Element, y: number) {
    const at = (x: number) => ({ pointerId: 1, pointerType: "mouse", pressure: 0.5, button: 0, buttons: 1, clientX: x, clientY: y });
    fireEvent.pointerDown(svg, at(10));
    fireEvent.pointerMove(svg, at(60));
    fireEvent.pointerMove(svg, at(120));
    fireEvent.pointerUp(svg, at(120));
  }

  it("Done adds the drawing as an ink block after the text; Discard adds nothing", async () => {
    createNote.mockResolvedValue(saved("N3", "Sketch"));
    const m = mount();
    m.type("Sketch");
    fireEvent.click(m.getByRole("button", { name: "Pen" }));
    let pen = m.getByRole("dialog", { name: "Pen" });
    expect(m.getByRole("button", { name: "Done" })).toBeDisabled();
    // The mouse and a finger draw on the pen's surface from the start.
    draw(m.getByRole("img", { name: "Drawing" }), 20);
    fireEvent.click(m.getByRole("button", { name: "Discard" }));
    expect(pen).not.toBeInTheDocument();
    expect(m.queryByRole("list", { name: "Drawings" })).toBeNull();

    fireEvent.click(m.getByRole("button", { name: "Pen" }));
    pen = m.getByRole("dialog", { name: "Pen" });
    draw(m.getByRole("img", { name: "Drawing" }), 20);
    draw(m.getByRole("img", { name: "Drawing" }), 40);
    fireEvent.click(m.getByRole("button", { name: "Done" }));
    expect(pen).not.toBeInTheDocument();
    expect(m.getByRole("list", { name: "Drawings" }).querySelectorAll("li").length).toBe(1);
    expect(m.getByRole("img", { name: "Drawing 1" })).toBeInTheDocument();

    fireEvent.click(m.save());
    await waitFor(() => expect(closeCapture).toHaveBeenCalled());
    const body = createNote.mock.calls[0]![1].blocks[0].blocks;
    expect(body).toHaveLength(2);
    expect(body[0]).toEqual({ kind: "paragraph", fields: { text: "Sketch" } });
    expect(body[1].kind).toBe("ink");
    expect(body[1].fields).toEqual({ height: 400 });
    expect(body[1].blocks[0].kind).toBe("viewport");
    expect(body[1].blocks.filter((b: { kind: string }) => b.kind === "stroke")).toHaveLength(2);
  });

  it("a drawing can be removed before saving, and a drawing alone is titled by the date", async () => {
    createNote.mockResolvedValue(saved("N4"));
    const m = mount();
    for (const _ of [1, 2]) {
      fireEvent.click(m.getByRole("button", { name: "Pen" }));
      draw(m.getByRole("img", { name: "Drawing" }), 20);
      fireEvent.click(m.getByRole("button", { name: "Done" }));
    }
    expect(m.getByRole("list", { name: "Drawings" }).querySelectorAll("li").length).toBe(2);
    expect(m.save()).toBeEnabled();
    fireEvent.click(m.getByRole("button", { name: "Remove drawing 1" }));
    expect(m.getByRole("list", { name: "Drawings" }).querySelectorAll("li").length).toBe(1);
    fireEvent.click(m.save());
    await waitFor(() => expect(closeCapture).toHaveBeenCalled());
    const doc = createNote.mock.calls[0]![1];
    expect(doc.fields.title).toMatch(/^Drawing \d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
    expect(doc.blocks[0].blocks.map((b: { kind: string }) => b.kind)).toEqual(["ink"]);
  });
});

describe("Send to", () => {
  const select = (m: ReturnType<typeof mount>) => m.getByRole("combobox", { name: "Send to" }) as HTMLSelectElement;

  it("lists No one, Assistant, then the enabled agents a fleeting note may call", () => {
    const m = mount();
    expect([...select(m).options].map((o) => o.textContent)).toEqual(["No one", "Assistant", "Clerk"]);
    expect(select(m).value).toBe("");
  });

  it("writes the call after the body, toasts, and remembers the choice", async () => {
    createNote.mockResolvedValue(saved("N5", "Fix the gate"));
    const m = mount();
    m.type("Fix the gate");
    fireEvent.change(select(m), { target: { value: "clerk" } });
    expect(localStorage.getItem(SEND_TO_KEY)).toBe("clerk");
    fireEvent.click(m.save());
    await waitFor(() => expect(closeCapture).toHaveBeenCalled());
    const body = createNote.mock.calls[0]![1].blocks[0].blocks;
    expect(body).toHaveLength(2);
    expect(body[0].fields.text).toBe("Fix the gate");
    expect(body[1].kind).toBe("paragraph");
    expect(body[1].fields.text).toMatch(new RegExp(`^\\[@Clerk Process this capture\\]\\(agent:clerk\\?call=${ULID.source}\\)$`));
    expect(showToast).toHaveBeenCalledWith("Saved and sent to Clerk", { label: "Open", href: "/notes/N5" });
    cleanup();

    // The next capture starts on the same choice, with a fresh call id.
    createNote.mockResolvedValue(saved("N6", "Again"));
    const again = mount();
    expect(select(again).value).toBe("clerk");
    again.type("Again");
    fireEvent.click(again.save());
    await waitFor(() => expect(createNote).toHaveBeenCalledTimes(2));
    const first = /call=([0-9A-Z]{26})/.exec(body[1].fields.text)![1];
    const second = /call=([0-9A-Z]{26})/.exec(createNote.mock.calls[1]![1].blocks[0].blocks[1].fields.text)![1];
    expect(second).not.toBe(first);
  });

  it("sends nothing with No one, and forgets an agent no longer listed", async () => {
    localStorage.setItem(SEND_TO_KEY, "gone");
    createNote.mockResolvedValue(saved("N7"));
    const m = mount();
    expect(select(m).value).toBe("");
    m.type("Plain");
    fireEvent.click(m.save());
    await waitFor(() => expect(closeCapture).toHaveBeenCalled());
    expect(createNote.mock.calls[0]![1].blocks[0].blocks).toHaveLength(1);
    expect(showToast).toHaveBeenCalledWith("Saved to inbox", expect.anything());
  });
});

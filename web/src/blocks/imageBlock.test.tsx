import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { type Asset, type Block, api } from "../api";
import { ImageBlock } from "./MediaBlocks";

const block = (fields: Block["fields"] = {}): Block => ({ kind: "image", fields: { asset: "IMG1", ...fields } });

function show(initial: Block, opts: { selected?: boolean; onRemove?: () => void } = {}) {
  const [b, setB] = createSignal(initial);
  const onChange = vi.fn((next: Block) => setB(next));
  const [selected, setSelected] = createSignal(opts.selected ?? false);
  const r = render(() => <ImageBlock block={b()} onChange={onChange} selected={selected()} onRemove={opts.onRemove ?? (() => {})} />);
  // The dialog is portalled to the body, so queries look at the whole document.
  return { ...r, ...screen, block: b, onChange, setSelected };
}

describe("an edited image block", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("shows just the image and its caption, with no inputs", () => {
    const { container } = show(block({ alt: "A cat", caption: "On the mat" }));
    expect(container.querySelector("img")?.getAttribute("src")).toBe("/api/assets/IMG1");
    expect(container.querySelector("figcaption")?.textContent).toBe("On the mat");
    expect(container.querySelector("input, select, textarea")).toBeNull();
    expect(container.querySelector("figure")?.classList.contains("selected")).toBe(false);
  });

  it("shows its Edit and Remove toolbar while selected", () => {
    const { container, getByLabelText, setSelected } = show(block());
    setSelected(true);
    expect(container.querySelector("figure")?.classList.contains("selected")).toBe(true);
    expect(getByLabelText("Edit image").tagName).toBe("BUTTON");
    expect(getByLabelText("Remove image").tagName).toBe("BUTTON");
  });

  it("removes the block with Remove", () => {
    const onRemove = vi.fn();
    const { getByLabelText, onChange } = show(block(), { selected: true, onRemove });
    fireEvent.click(getByLabelText("Remove image"));
    expect(onRemove).toHaveBeenCalledTimes(1);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("opens the Image dialog with Edit, and Save applies alt text, caption and width as one change", () => {
    const { getByLabelText, getByRole, queryByRole, onChange, block: current } = show(block({ alt: "Old" }), { selected: true });
    fireEvent.click(getByLabelText("Edit image"));
    getByRole("dialog", { name: "Image" });
    const alt = getByLabelText("Alt text") as HTMLInputElement;
    expect(alt.value).toBe("Old");
    expect(document.activeElement).toBe(alt);
    fireEvent.input(alt, { target: { value: "A red square" } });
    fireEvent.input(getByLabelText("Caption"), { target: { value: "Two pixels wide" } });
    fireEvent.input(getByLabelText("Width"), { target: { value: "320" } });
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(getByRole("button", { name: "Save" }));
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(current().fields).toEqual({ asset: "IMG1", alt: "A red square", caption: "Two pixels wide", width: 320 });
    expect(queryByRole("dialog")).toBeNull();
  });

  it("clears the width when it is emptied, for the content width", () => {
    const { getByLabelText, getByRole, block: current } = show(block({ width: 200 }), { selected: true });
    fireEvent.click(getByLabelText("Edit image"));
    fireEvent.input(getByLabelText("Width"), { target: { value: "" } });
    fireEvent.click(getByRole("button", { name: "Save" }));
    expect(current().fields).toEqual({ asset: "IMG1" });
  });

  it("changes nothing on Cancel or Escape", () => {
    const { getByLabelText, getByRole, queryByRole, onChange } = show(block({ alt: "Old" }), { selected: true });
    fireEvent.click(getByLabelText("Edit image"));
    fireEvent.input(getByLabelText("Alt text"), { target: { value: "New" } });
    fireEvent.click(getByRole("button", { name: "Cancel" }));
    expect(queryByRole("dialog")).toBeNull();

    fireEvent.click(getByLabelText("Edit image"));
    expect((getByLabelText("Alt text") as HTMLInputElement).value).toBe("Old");
    fireEvent.input(getByLabelText("Alt text"), { target: { value: "New" } });
    fireEvent.keyDown(getByRole("dialog"), { key: "Escape" });
    expect(queryByRole("dialog")).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("uploads a replacement with Replace image and sets the asset on Save", async () => {
    const uploaded: Asset = { id: "IMG2", filename: "new.png", size: 1, path: "assets/IMG2/new.png" };
    const uploadAsset = vi.spyOn(api, "uploadAsset").mockResolvedValue(uploaded);
    const { getByLabelText, getByRole, onChange, block: current } = show(block({ caption: "Kept" }), { selected: true });
    fireEvent.click(getByLabelText("Edit image"));
    const dialog = getByRole("dialog", { name: "Image" });
    const input = dialog.querySelector<HTMLInputElement>("input[type=file]")!;
    Object.defineProperty(input, "files", { value: [new File(["x"], "new.png", { type: "image/png" })] });
    fireEvent.change(input);
    await waitFor(() => expect(dialog.querySelector("img.image-dialog-preview")?.getAttribute("src")).toBe("/api/assets/IMG2"));
    expect(uploadAsset).toHaveBeenCalledTimes(1);
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(getByRole("button", { name: "Save" }));
    expect(current().fields).toEqual({ asset: "IMG2", caption: "Kept" });
  });

  it("shows an upload error in the dialog", async () => {
    vi.spyOn(api, "uploadAsset").mockRejectedValue(new Error("too large"));
    const { getByLabelText, getByRole, onChange } = show(block(), { selected: true });
    fireEvent.click(getByLabelText("Edit image"));
    const dialog = getByRole("dialog", { name: "Image" });
    const input = dialog.querySelector<HTMLInputElement>("input[type=file]")!;
    Object.defineProperty(input, "files", { value: [new File(["x"], "big.png", { type: "image/png" })] });
    fireEvent.change(input);
    await waitFor(() => expect(dialog.querySelector("[role=alert]")?.textContent).toBe("Could not upload big.png: too large"));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("is unchanged where it cannot be edited", () => {
    const { container } = render(() => <ImageBlock block={block({ caption: "Read only" })} selected />);
    expect(container.querySelector(".block-image-tools")).toBeNull();
    expect(container.querySelector("figcaption")?.textContent).toBe("Read only");
  });
});

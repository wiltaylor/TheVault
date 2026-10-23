import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type Asset, type Block, type FieldDef, type Json, type TypeDef, api } from "../api";
import { FieldRefs } from "../components/fieldRefs";
import { type ControlBinding, ControlBindingContext } from "../controls/binding";
import { ImageBlock, isImageFile } from "./MediaBlocks";

const asset = (id: string, filename: string): Asset => ({ id, filename, size: 10, path: `assets/${id}/${filename}` });
const ASSETS = [asset("IMG1", "one.png"), asset("IMG2", "two.JPG"), asset("DOC1", "notes.pdf"), asset("IMG3", "three.webp")];

const TYPE = {
  name: "album",
  label: "Album",
  fields: [
    { name: "cover", label: "Cover", kind: "asset" },
    { name: "photos", label: "Photos", kind: "asset", multiple: true },
  ],
  templates: [],
} as unknown as TypeDef;

function binding(values: Record<string, Json>, set?: (f: FieldDef, v: Json | undefined) => void): ControlBinding {
  return { type: () => TYPE, value: (f) => values[f.name], set };
}

const image = (field: string, extra: Record<string, Json> = {}): Block => ({ kind: "image", fields: { field, ...extra } });

function show(block: Block, b: ControlBinding, onChange?: (b: Block) => void) {
  return render(() => (
    <ControlBindingContext.Provider value={b}>
      <FieldRefs.Provider value={{ text: (n) => (n === "title" ? "Blue Train" : null) }}>
        <ImageBlock block={block} onChange={onChange} />
      </FieldRefs.Provider>
    </ControlBindingContext.Provider>
  ));
}

const srcs = (c: HTMLElement) => [...c.querySelectorAll("img")].map((i) => i.getAttribute("src"));

describe("an image from an asset field", () => {
  beforeEach(() => {
    vi.spyOn(api, "assets").mockResolvedValue(ASSETS);
  });
  afterEach(() => vi.restoreAllMocks());

  it("knows images by their file name", () => {
    expect(isImageFile("a.png")).toBe(true);
    expect(isImageFile("a.JPEG")).toBe(true);
    expect(isImageFile("a.svg")).toBe(true);
    expect(isImageFile("a.pdf")).toBe(false);
    expect(isImageFile("png")).toBe(false);
  });

  it("shows the image in a single field, with alt text and caption filled in", async () => {
    const { container } = show(image("cover", { alt: "Cover of {{title}}", caption: "{{title}} sleeve", width: 200 }), binding({ cover: "IMG1" }));
    await waitFor(() => expect(srcs(container)).toEqual(["/api/assets/IMG1"]));
    const img = container.querySelector("img")!;
    expect(img.getAttribute("alt")).toBe("Cover of Blue Train");
    expect(img.getAttribute("width")).toBe("200");
    expect(container.querySelector("figcaption")?.textContent).toBe("Blue Train sleeve");
  });

  it("shows every image in a multiple field, in order, and skips files that are not images", async () => {
    const { container } = show(image("photos"), binding({ photos: ["IMG2", "DOC1", "IMG3"] }));
    await waitFor(() => expect(srcs(container)).toEqual(["/api/assets/IMG2", "/api/assets/IMG3"]));
    expect(container.querySelector(".field-images")).not.toBeNull();
  });

  it("shows nothing for an empty field where the note cannot be edited", async () => {
    const { container } = show(image("cover"), binding({}));
    await new Promise((r) => setTimeout(r, 20));
    expect(container.querySelector("figure")).toBeNull();
    expect(container.querySelector("button")).toBeNull();
  });

  it("shows nothing for a field holding only files that are not images", async () => {
    const { container } = show(image("photos"), binding({ photos: ["DOC1"] }));
    await new Promise((r) => setTimeout(r, 20));
    expect(srcs(container)).toEqual([]);
  });

  it("offers to add an image to an empty field on the note page, and sets the field", async () => {
    const uploaded = asset("NEW1", "new.png");
    vi.spyOn(api, "uploadAsset").mockResolvedValue(uploaded);
    const [values, setValues] = createSignal<Record<string, Json>>({});
    const set = vi.fn((f: FieldDef, v: Json | undefined) => setValues({ ...values(), [f.name]: v as Json }));
    const b: ControlBinding = { type: () => TYPE, value: (f) => values()[f.name], set };
    const { container, getByTestId } = show(image("photos"), b);
    const add = getByTestId("field-image-add");
    expect(add.textContent).toContain("Add image");
    const input = container.querySelector<HTMLInputElement>("input[type=file]")!;
    const file = new File(["x"], "new.png", { type: "image/png" });
    Object.defineProperty(input, "files", { value: [file] });
    fireEvent.change(input);
    await waitFor(() => expect(set).toHaveBeenCalled());
    // A multiple field takes the image as a list.
    expect(set.mock.calls[0]![1]).toEqual(["NEW1"]);
    await waitFor(() => expect(srcs(container)).toEqual(["/api/assets/NEW1"]));
  });

  it("is a placeholder naming the field in the template editor, whose field the Edit dialog changes", () => {
    const [block, setBlock] = createSignal<Block>(image("cover"));
    render(() => (
      <ControlBindingContext.Provider value={{ ...binding({ cover: "IMG1" }), preview: true }}>
        <ImageBlock block={block()} onChange={setBlock} selected onRemove={() => {}} />
      </ControlBindingContext.Provider>
    ));
    // The dialog is portalled to the body, so queries look at the whole document.
    const { getByTestId, getByLabelText, getByRole, queryByLabelText } = screen;
    expect(getByTestId("field-image-placeholder").textContent).toBe("Image from Cover");
    expect(queryByLabelText("Image field")).toBeNull();
    expect(queryByLabelText("Alt text")).toBeNull();
    fireEvent.click(getByLabelText("Edit image"));
    const dialog = getByRole("dialog", { name: "Image" });
    const select = getByLabelText("Image field") as HTMLSelectElement;
    expect([...select.options].map((o) => o.value)).toEqual(["cover", "photos"]);
    // A field image has no file of its own to replace.
    expect(dialog.querySelector('[aria-label="Replace image"]')).toBeNull();
    fireEvent.change(select, { target: { value: "photos" } });
    expect(block().fields?.field).toBe("cover");
    fireEvent.click(getByRole("button", { name: "Save" }));
    expect(block().fields?.field).toBe("photos");
    expect(getByTestId("field-image-placeholder").textContent).toBe("Image from Photos");
  });

  describe("empty on the note page", () => {
    const typed = (items: unknown[]): TypeDef =>
      ({ ...TYPE, templates: [{ name: "default", tabs: [{ title: "Main", items }] }] }) as unknown as TypeDef;
    const editable = (type: TypeDef): ControlBinding => ({ type: () => type, value: () => undefined, set: vi.fn() });

    it("shows nothing when the template also places the field with a slot", async () => {
      const type = typed([{ block: image("photos") }, { slot: "photos" }]);
      const { container } = show(image("photos"), editable(type));
      await new Promise((r) => setTimeout(r, 20));
      expect(container.querySelector("figure")).toBeNull();
      expect(container.querySelector("button")).toBeNull();
    });

    it("still offers the row when the only other place is a control that cannot edit an asset field", () => {
      // No control kind edits an asset field, so such a control uploads nothing and places nothing.
      const control: Block = { kind: "control", fields: { field: "photos", kind: "text" } };
      const type = typed([{ block: image("photos") }, { block: control }]);
      const { getByTestId } = show(image("photos"), editable(type));
      expect(getByTestId("field-image-add")).toBeTruthy();
    });

    it("offers a compact add-image row when the template places the field nowhere else", () => {
      const type = typed([{ block: image("photos") }, { slot: "title" }]);
      const { getByTestId } = show(image("photos"), editable(type));
      const add = getByTestId("field-image-add");
      expect(add.classList.contains("add")).toBe(true);
      expect(add.getAttribute("aria-label")).toBe("Add image to Photos");
      expect(add.textContent).toBe("Add image");
    });

    it("grows into a drop target while a file is dragged over it, and uploads a dropped image", async () => {
      vi.spyOn(api, "uploadAsset").mockResolvedValue(asset("NEW2", "dropped.png"));
      const set = vi.fn();
      const type = typed([{ block: image("cover") }]);
      const { getByTestId } = show(image("cover"), { type: () => type, value: () => undefined, set });
      const add = getByTestId("field-image-add");
      const file = new File(["x"], "dropped.png", { type: "image/png" });
      const dataTransfer = { types: ["Files"], files: [file], dropEffect: "none" };
      fireEvent.dragEnter(add, { dataTransfer });
      expect(add.classList.contains("over")).toBe(true);
      expect(add.textContent).toBe("Drop image to add it");
      fireEvent.drop(add, { dataTransfer });
      expect(add.classList.contains("over")).toBe(false);
      await waitFor(() => expect(set).toHaveBeenCalled());
      // A single field takes the image's id.
      expect(set.mock.calls[0]![1]).toBe("NEW2");
    });
  });
});

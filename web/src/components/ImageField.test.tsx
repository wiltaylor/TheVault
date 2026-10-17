import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import type { FieldDef, Json } from "../api";

const FACE = "01JB9A0C3D4E5F6G7H8J9K0M1N";
const BEACH = "01JB9A0C3D4E5F6G7H8J9K0M1P";
const NEW = "01JB9A0C3D4E5F6G7H8J9K0M1Q";

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  const summary = (id: string, filename: string) => ({
    id,
    filename,
    size: 3,
    path: `assets/${id}/${filename}`,
    kind: "image",
    content_type: "image/png",
    uploaded_ms: 0,
    used_by: 0,
    notes: 0,
    trashed: 0,
    types: 0,
  });
  return {
    ...real,
    api: {
      ...real.api,
      assetLibrary: vi.fn(async (q: { q?: string }) => ({
        assets: [summary(FACE, "face.png"), summary(BEACH, "beach.jpg")].filter((a) => !q.q || a.filename.includes(q.q)),
        total: 2,
        next: null,
      })),
      assets: vi.fn(async () => []),
      uploadAsset: vi.fn(async (file: File) => ({ id: NEW, filename: file.name, size: file.size, path: "" })),
    },
  };
});

import { api } from "../api";
import { FieldInput } from "./FieldInput";

const PHOTO: FieldDef = {
  name: "photo",
  label: "Photo",
  description: null,
  kind: "asset",
  accept: "image",
  required: false,
  computed: false,
  multiple: false,
  compound: false,
};

function mount(start?: string, field: FieldDef = PHOTO) {
  const [value, setValue] = createSignal<Json | undefined>(start);
  const r = render(() => <FieldInput field={field} value={value()} onChange={setValue} label={field.label} id="photo" />);
  return { ...r, value };
}

describe("image field", () => {
  it("shows a placeholder when empty: Add photo for a Photo field, else Add image", () => {
    const m = mount();
    const tile = m.getByRole("button", { name: "Add photo" });
    expect(tile.classList.contains("empty")).toBe(true);
    expect(tile.querySelector("img")).toBeNull();
    m.unmount();
    const other = mount(undefined, { ...PHOTO, name: "cover", label: "Cover" });
    expect(other.getByRole("button", { name: "Add image" })).toBeTruthy();
  });

  it("picks an existing image asset from the dialog", async () => {
    const m = mount();
    fireEvent.click(m.getByRole("button", { name: "Add photo" }));
    const dialog = m.getByRole("dialog", { name: "Choose Photo" });
    expect(dialog).toBeTruthy();
    await waitFor(() => expect(m.getByRole("button", { name: "beach.jpg" })).toBeTruthy());
    // Search narrows the grid.
    const search = m.getByRole("searchbox", { name: "Search images" }) as HTMLInputElement;
    search.value = "face";
    fireEvent.input(search);
    await waitFor(() => expect(m.queryByRole("button", { name: "beach.jpg" })).toBeNull());
    fireEvent.click(m.getByRole("button", { name: "face.png" }));
    expect(m.value()).toBe(FACE);
    expect(m.queryByRole("dialog")).toBeNull();
    const img = m.getByRole("button", { name: "Change Photo" }).querySelector("img")!;
    expect(img.getAttribute("src")).toBe(`/api/assets/${FACE}`);
  });

  it("uploads an image into the field, and refuses a file that is not one", async () => {
    const m = mount();
    fireEvent.click(m.getByRole("button", { name: "Add photo" }));
    const input = m.getByRole("dialog").querySelector<HTMLInputElement>("input[type=file]")!;
    expect(input.accept).toBe("image/*");
    const pdf = new File(["x"], "cv.pdf", { type: "application/pdf" });
    fireEvent.drop(m.getByRole("dialog"), { dataTransfer: { files: [pdf], types: ["Files"] } });
    await waitFor(() => expect(m.getByRole("alert").textContent).toContain("cv.pdf is not an image"));
    expect(api.uploadAsset).not.toHaveBeenCalled();
    const png = new File(["x"], "me.png", { type: "image/png" });
    Object.defineProperty(input, "files", { value: [png], configurable: true });
    fireEvent.change(input);
    await waitFor(() => expect(m.value()).toBe(NEW));
    expect(api.uploadAsset).toHaveBeenCalledWith(png);
  });

  it("offers Remove and Open full size once set, and Remove clears it", () => {
    const m = mount(FACE);
    const open = m.getByRole("link", { name: "Open full size" });
    expect(open.getAttribute("href")).toBe(`/api/assets/${FACE}`);
    fireEvent.click(m.getByRole("button", { name: "Remove" }));
    expect(m.value()).toBeUndefined();
    expect(m.getByRole("button", { name: "Add photo" })).toBeTruthy();
  });

  it("keeps the asset picker for a field that takes any asset", () => {
    const m = mount(undefined, { ...PHOTO, accept: "any" });
    expect(m.queryByTestId("image-field")).toBeNull();
    expect(m.container.querySelector(".asset-input")).toBeTruthy();
  });
});

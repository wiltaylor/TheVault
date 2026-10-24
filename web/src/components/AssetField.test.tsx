import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Asset, FieldDef, Json } from "../api";

const DOC = "01JB9A0C3D4E5F6G7H8J9K0M1A";
const PIC = "01JB9A0C3D4E5F6G7H8J9K0M1B";
const PDF = "01JB9A0C3D4E5F6G7H8J9K0M1C";
const OLD = "01JB9A0C3D4E5F6G7H8J9K0M1D";
const GONE = "01JB9A0C3D4E5F6G7H8J9K0M1Z";

const asset = (id: string, filename: string, size = 2048): Asset => ({ id, filename, size, path: `assets/${id}/${filename}` });
const LISTING = [asset(DOC, "notes.txt", 12), asset(PIC, "beach.png"), asset(PDF, "report.pdf"), asset(OLD, "old-budget.xlsx")];

vi.mock("../pdf/load", () => ({
  pdfLib: () => Promise.resolve({ thumbnail: async () => "data:image/png;base64," }),
}));

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  const summary = (a: Asset) => ({ ...a, kind: "other", content_type: "", uploaded_ms: 0, used_by: 0, notes: 0, trashed: 0, types: 0 });
  let next = 0;
  return {
    ...real,
    api: {
      ...real.api,
      assets: vi.fn(async () => LISTING),
      // Newest first, as the server sorts by upload time.
      assetLibrary: vi.fn(async (q: { q?: string }) => ({
        assets: [...LISTING]
          .reverse()
          .filter((a) => !q.q || a.filename.includes(q.q))
          .map(summary),
        total: LISTING.length,
        next: null,
      })),
      uploadAsset: vi.fn(async (file: File) => {
        if (file.name.startsWith("bad")) throw new Error("too big");
        next += 1;
        return { id: `01JB9A0C3D4E5F6G7H8J9K0N${String(next).padStart(2, "0")}`, filename: file.name, size: file.size, path: "" };
      }),
    },
  };
});

import { api } from "../api";
import { AssetInput } from "./AssetField";
import { FieldInput } from "./FieldInput";

const FILES: FieldDef = { name: "files", label: "Files", kind: "asset", multiple: true, compound: false, required: false, computed: false, description: null };
const COVER: FieldDef = { name: "cover", label: "Cover", kind: "asset", multiple: false, compound: false, required: false, computed: false, description: null };

function mount(field: FieldDef, initial: Json | undefined) {
  const [value, setValue] = createSignal<Json | undefined>(initial);
  const r = render(() => <FieldInput field={field} value={value()} onChange={setValue} label={field.label} id="f" />);
  return { ...r, value };
}

const names = (c: HTMLElement) => [...c.querySelectorAll(".asset-row .asset-row-name")].map((e) => e.textContent);

function openMenu(m: ReturnType<typeof mount>, label: string) {
  fireEvent.click(m.getByRole("button", { name: `Add an attachment to ${label}` }));
  return m.getByRole("menu");
}

function pickFiles(m: ReturnType<typeof mount>, files: File[]) {
  const input = m.getByTestId("asset-file-input") as HTMLInputElement;
  Object.defineProperty(input, "files", { value: files, configurable: true });
  fireEvent.change(input);
}

afterEach(() => vi.clearAllMocks());

describe("an asset field", () => {
  it("lists each file with its icon, a link to its page, its size and a remove button", async () => {
    const m = mount(FILES, [DOC, PIC, PDF]);
    await waitFor(() => expect(names(m.container)).toEqual(["notes.txt", "beach.png", "report.pdf"]));
    const link = m.getByRole("link", { name: "notes.txt" });
    expect(link.getAttribute("href")).toBe(`/assets/${DOC}`);
    expect(m.getByText("12 B")).toBeTruthy();
    expect(m.getAllByText("2.0 KB")).toHaveLength(2);
    // An image shows itself, a PDF its first page (opening the viewer), anything else a file icon.
    const rows = m.container.querySelectorAll(".asset-row");
    expect(rows[0]!.querySelector(".asset-row-icon svg")).toBeTruthy();
    expect(rows[1]!.querySelector(".asset-row-icon img")!.getAttribute("src")).toBe(`/api/assets/${PIC}`);
    expect(m.getByRole("button", { name: "View report.pdf" })).toBeTruthy();
    for (const n of ["notes.txt", "beach.png", "report.pdf"]) expect(m.getByRole("button", { name: `Remove ${n}` })).toBeTruthy();
  });

  it("removes a file from the field, leaving no value once the last is gone", async () => {
    const m = mount(FILES, [DOC, PIC]);
    fireEvent.click(await m.findByRole("button", { name: "Remove notes.txt" }));
    expect(m.value()).toEqual([PIC]);
    fireEvent.click(m.getByRole("button", { name: "Remove beach.png" }));
    expect(m.value()).toBeUndefined();
  });

  it("keeps a stored id the listing lacks, shown by its id and removable", async () => {
    const m = mount(FILES, [GONE, DOC]);
    await waitFor(() => expect(names(m.container)).toEqual([GONE, "notes.txt"]));
    expect(m.container.querySelector(".asset-row.missing")).toBeTruthy();
    fireEvent.click(m.getByRole("button", { name: `Remove ${GONE}` }));
    expect(m.value()).toEqual([DOC]);
  });

  it("Add + opens a menu that closes on Escape, handing focus back, and on a click outside", async () => {
    const m = mount(FILES, undefined);
    const add = m.getByRole("button", { name: "Add an attachment to Files" });
    expect(add.getAttribute("aria-expanded")).toBe("false");
    const menu = openMenu(m, "Files");
    expect(add.getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement).toBe(m.getByRole("menuitem", { name: "Upload a file…" }));
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(m.getByRole("menuitem", { name: "Choose from the vault…" }));
    fireEvent.keyDown(menu, { key: "Escape" });
    expect(m.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(add);
    openMenu(m, "Files");
    fireEvent.pointerDown(document.body);
    expect(m.queryByRole("menu")).toBeNull();
  });

  it("Upload uploads several files at once into a multiple field, appending them, and reports a failure inline", async () => {
    const m = mount(FILES, [DOC]);
    openMenu(m, "Files");
    fireEvent.click(m.getByRole("menuitem", { name: "Upload a file…" }));
    expect(m.queryByRole("menu")).toBeNull();
    expect((m.getByTestId("asset-file-input") as HTMLInputElement).multiple).toBe(true);
    pickFiles(m, [new File(["a"], "a.txt"), new File(["b"], "bad.bin"), new File(["cc"], "c.txt")]);
    expect(m.getByRole("status").textContent).toContain("of 3");
    await waitFor(() => expect((m.value() as string[]).length).toBe(3));
    expect(api.uploadAsset).toHaveBeenCalledTimes(3);
    await waitFor(() => expect(names(m.container)).toEqual(["notes.txt", "a.txt", "c.txt"]));
    expect(m.getByRole("alert").textContent).toBe("Could not upload bad.bin: too big");
    expect(m.queryByRole("status")).toBeNull();
  });

  it("Upload into a single field replaces its file, and the menu says so", async () => {
    const m = mount(COVER, DOC);
    await waitFor(() => expect(names(m.container)).toEqual(["notes.txt"]));
    const menu = openMenu(m, "Cover");
    expect(menu.textContent).toContain("Replace notes.txt");
    fireEvent.click(m.getByRole("menuitem", { name: "Upload a file…" }));
    expect((m.getByTestId("asset-file-input") as HTMLInputElement).multiple).toBe(false);
    pickFiles(m, [new File(["x"], "cover.txt")]);
    await waitFor(() => expect(typeof m.value() === "string" && m.value() !== DOC).toBe(true));
    await waitFor(() => expect(names(m.container)).toEqual(["cover.txt"]));
  });

  it("uploads files dropped on the field", async () => {
    const m = mount(FILES, undefined);
    const box = m.getByTestId("asset-input");
    const files = [new File(["d"], "dropped.txt")];
    fireEvent.dragOver(box, { dataTransfer: { types: ["Files"], files } });
    expect(box.classList.contains("dragging")).toBe(true);
    fireEvent.drop(box, { dataTransfer: { types: ["Files"], files } });
    await waitFor(() => expect((m.value() as string[] | undefined)?.length).toBe(1));
    expect(box.classList.contains("dragging")).toBe(false);
    await waitFor(() => expect(names(m.container)).toEqual(["dropped.txt"]));
  });

  it("Choose from the vault lists files newest first, searches them, and adds every one ticked", async () => {
    const m = mount(FILES, [DOC]);
    openMenu(m, "Files");
    fireEvent.click(m.getByRole("menuitem", { name: "Choose from the vault…" }));
    const dialog = m.getByRole("dialog", { name: "Choose from the vault for Files" });
    const items = () => [...dialog.querySelectorAll(".asset-chooser-item")].map((b) => b.getAttribute("aria-label"));
    await waitFor(() => expect(items()).toEqual(["old-budget.xlsx", "report.pdf", "beach.png", "notes.txt"]));
    // The file already attached is marked and cannot be added again.
    expect((m.getByRole("button", { name: "notes.txt" }) as HTMLButtonElement).disabled).toBe(true);
    const search = m.getByRole("searchbox", { name: "Search files" });
    expect(document.activeElement).toBe(search);
    fireEvent.input(search, { target: { value: "e" } });
    await waitFor(() => expect(api.assetLibrary).toHaveBeenLastCalledWith(expect.objectContaining({ q: "e", sort: "uploaded", dir: "desc" })));
    fireEvent.input(search, { target: { value: "port" } });
    await waitFor(() => expect(items()).toEqual(["report.pdf"]));
    fireEvent.click(m.getByRole("button", { name: "report.pdf" }));
    fireEvent.input(search, { target: { value: "" } });
    await waitFor(() => expect(items()).toHaveLength(4));
    fireEvent.click(m.getByRole("button", { name: "beach.png" }));
    fireEvent.click(m.getByRole("button", { name: "Add 2 files" }));
    expect(m.queryByRole("dialog")).toBeNull();
    expect(m.value()).toEqual([DOC, PDF, PIC]);
  });

  it("Choose from the vault into a single field takes the one clicked, replacing its file", async () => {
    const m = mount(COVER, DOC);
    openMenu(m, "Cover");
    fireEvent.click(m.getByRole("menuitem", { name: "Choose from the vault…" }));
    fireEvent.click(await m.findByRole("button", { name: "old-budget.xlsx" }));
    expect(m.queryByRole("dialog")).toBeNull();
    expect(m.value()).toBe(OLD);
    await waitFor(() => expect(names(m.container)).toEqual(["old-budget.xlsx"]));
  });

  it("closes the vault dialog on Escape without changing the field", async () => {
    const m = mount(FILES, [DOC]);
    openMenu(m, "Files");
    fireEvent.click(m.getByRole("menuitem", { name: "Choose from the vault…" }));
    fireEvent.keyDown(m.getByRole("dialog"), { key: "Escape" });
    expect(m.queryByRole("dialog")).toBeNull();
    expect(m.value()).toEqual([DOC]);
  });

  it("shows only the first file of a single field holding a list", async () => {
    const m = mount(COVER, [DOC, PIC]);
    await waitFor(() => expect(names(m.container)).toEqual(["notes.txt"]));
  });

  it("read-only, lists the files without remove or Add", async () => {
    const r = render(() => <AssetInput field={FILES} value={[DOC, GONE]} label="Files" />);
    await waitFor(() => expect(names(r.container)).toEqual(["notes.txt", GONE]));
    expect(r.queryByRole("button", { name: /Remove/ })).toBeNull();
    expect(r.queryByRole("button", { name: /Add an attachment/ })).toBeNull();
    expect(r.getByRole("link", { name: "notes.txt" })).toBeTruthy();
  });

  it("read-only and empty, shows a dash", () => {
    const r = render(() => <AssetInput field={FILES} value={undefined} label="Files" />);
    expect(r.container.textContent).toBe("—");
  });
});

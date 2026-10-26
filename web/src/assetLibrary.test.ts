import { beforeEach, describe, expect, it } from "vitest";
import type { TypeDef } from "./api";
import {
  ASSET_FILTERS,
  assetIdOf,
  assetName,
  extension,
  howText,
  kindIcon,
  kindLabel,
  kindOf,
  knowAsset,
  loadAssetsView,
  saveAssetsView,
  sizeText,
  usedText,
} from "./assetLibrary";
import { TYPE_VIEWS_KEY, loadTypeView } from "./typeList";

const ID = "01JB9A0C3D4E5F6G7H8J9K0M1N";

describe("the assets library model", () => {
  beforeEach(() => localStorage.clear());

  it("remembers its view beside the types' views, under a key no type can have", () => {
    expect(loadAssetsView()).toBe("cards");
    saveAssetsView("list");
    expect(loadAssetsView()).toBe("list");
    expect(JSON.parse(localStorage.getItem(TYPE_VIEWS_KEY)!)).toEqual({ ":assets": "list" });
    expect(loadTypeView("assets")).toBe("cards");
    saveAssetsView("cards");
    expect(loadAssetsView()).toBe("cards");
  });

  it("reads sizes, kinds and uses", () => {
    expect(sizeText(512)).toBe("512 B");
    expect(sizeText(2048)).toBe("2.0 KB");
    expect(sizeText(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(extension("photo.JPG")).toBe("JPG");
    expect(extension("README")).toBe("");
    expect(kindLabel("photo.png", "image")).toBe("PNG image");
    expect(kindLabel("report.pdf", "pdf")).toBe("PDF");
    expect(kindLabel("song.mp3", "audio")).toBe("MP3 audio");
    expect(kindLabel("export.zip", "other")).toBe("ZIP file");
    expect(kindLabel("Plan.docx", "document")).toBe("DOCX document");
    expect(kindLabel("budget.xlsx", "spreadsheet")).toBe("XLSX spreadsheet");
    expect(kindLabel("people.csv", "spreadsheet")).toBe("CSV spreadsheet");
    expect(kindLabel("README", "other")).toBe("File");
    expect(kindOf("a.webp")).toBe("image");
    expect(kindOf("a.PDF")).toBe("pdf");
    expect(kindOf("a.mp4")).toBe("video");
    expect(kindOf("a.bin")).toBe("other");
    for (const name of ["a.docx", "a.DOC", "a.odt"]) expect(kindOf(name)).toBe("document");
    for (const name of ["a.xlsx", "a.xls", "a.ods", "a.CSV"]) expect(kindOf(name)).toBe("spreadsheet");
    expect(kindIcon("document")).toBe("file-pen-line");
    expect(kindIcon("spreadsheet")).toBe("file-spreadsheet");
    expect(kindIcon("pdf")).toBe("file-text");
    expect(kindIcon(undefined)).toBe("file");
    expect(usedText(0)).toBe("Not used");
    expect(usedText(1)).toBe("Used by 1 note");
    expect(usedText(3)).toBe("Used by 3 notes");
  });

  it("filters by documents and spreadsheets beside images and PDFs, as the server counts them", () => {
    expect(ASSET_FILTERS.map((f) => f.id)).toEqual(["all", "image", "pdf", "document", "spreadsheet", "other", "unused"]);
    expect(ASSET_FILTERS.map((f) => f.label)).toEqual(["All", "Images", "PDFs", "Documents", "Spreadsheets", "Other", "Unused"]);
  });

  it("says how a note uses an asset, with the field's label", () => {
    const type = { name: "doc", fields: [{ name: "cover", label: "Cover photo" }, { name: "body", label: "Body" }] } as unknown as TypeDef;
    expect(howText({ field: "cover", via: "field" }, type)).toBe("Cover photo");
    expect(howText({ field: "body", via: "image" }, type)).toBe("Image in Body");
    expect(howText({ field: "body", via: "asset" }, type)).toBe("File in Body");
    expect(howText({ field: "board", via: "node" }, undefined)).toBe("Card in board");
  });

  it("finds the asset an address shows and remembers its name", () => {
    expect(assetIdOf(`/assets/${ID}`)).toBe(ID);
    expect(assetIdOf(`/assets/${ID.toLowerCase()}?x=1`)).toBe(ID);
    expect(assetIdOf("/assets")).toBeNull();
    expect(assetIdOf("/assets/trash")).toBeNull();
    expect(assetName(ID)).toBeUndefined();
    knowAsset(ID, "photo.png");
    expect(assetName(ID)).toBe("photo.png");
  });
});

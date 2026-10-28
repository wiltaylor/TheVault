import { render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Block } from "../api";
import { Blocks } from "../components/Render";
import { nestedText } from "./toggle";

const table: Block = {
  kind: "table",
  fields: { header: true, align: ["left", "right"] },
  blocks: [
    { kind: "row", fields: { cells: ["Item", "Qty"] } },
    { kind: "row", fields: { cells: ["**Apples**", "3"] } },
    { kind: "row", fields: { cells: ["Pears"] } },
  ],
};

describe("a table, read-only", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("is a table with its header row, alignments and rendered markup, short rows padded", () => {
    const { container } = render(() => <Blocks blocks={[table]} />);
    const el = container.querySelector("[data-testid='table-block'] table")!;
    expect([...el.querySelectorAll("thead th")].map((th) => th.textContent)).toEqual(["Item", "Qty"]);
    const rows = [...el.querySelectorAll("tbody tr")].map((tr) => [...tr.querySelectorAll("td")].map((td) => td.textContent));
    expect(rows).toEqual([
      ["Apples", "3"],
      ["Pears", ""],
    ]);
    expect(el.querySelector("tbody td strong")?.textContent).toBe("Apples");
    expect((el.querySelector("thead th:nth-child(2)") as HTMLElement).style.textAlign).toBe("right");
    expect((el.querySelector("tbody td:first-child") as HTMLElement).style.textAlign).toBe("");
  });

  it("has no header row unless the block says so", () => {
    const { container } = render(() => <Blocks blocks={[{ ...table, fields: {} }]} />);
    expect(container.querySelector("thead")).toBeNull();
    expect(container.querySelectorAll("tbody tr")).toHaveLength(3);
  });

  it("copies as a Markdown table", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    vi.stubGlobal("isSecureContext", true);
    const { getByText } = render(() => <Blocks blocks={[table]} />);
    getByText("Copy as Markdown").click();
    await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith("| Item | Qty |\n| --- | ---: |\n| **Apples** | 3 |\n| Pears |  |"));
  });

  it("is searched inside a toggle by its cells' text", () => {
    expect(nestedText([table])).toContain("apples");
  });
});

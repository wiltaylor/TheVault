import { describe, expect, it } from "vitest";
import {
  markdownPieces,
  markdownToMarkup,
  markupToMarkdown,
  newTable,
  splitRow,
  tableBlock,
  tableOf,
  tableToMarkdown,
  type TableData,
} from "./table";

const ID = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";

/** The one table in `md`. */
function tableIn(md: string): TableData {
  const pieces = markdownPieces(md);
  const tables = (pieces ?? []).flatMap((p) => (p.kind === "table" ? [p.table] : []));
  expect(tables).toHaveLength(1);
  return tables[0]!;
}

describe("a table block", () => {
  it("reads square: short rows padded, an alignment per column, left by default", () => {
    const t = tableOf({
      kind: "table",
      fields: { header: true, align: ["right", "sideways"] },
      blocks: [
        { kind: "row", fields: { cells: ["a", "b", "c"] } },
        { kind: "row", fields: { cells: ["d"] } },
      ],
    });
    expect(t).toEqual({ header: true, align: ["right", "left", "left"], rows: [["a", "b", "c"], ["d", "", ""]] });
  });

  it("writes every row as wide as the widest, and align only when a column is not left", () => {
    expect(tableBlock({ header: false, align: ["left"], rows: [["a", "b"], ["c"]] })).toEqual({
      kind: "table",
      blocks: [
        { kind: "row", fields: { cells: ["a", "b"] } },
        { kind: "row", fields: { cells: ["c", ""] } },
      ],
    });
    expect(tableBlock({ header: true, align: ["left", "center"], rows: [["a", "b"]] }).fields).toEqual({ header: true, align: ["left", "center"] });
    expect(tableBlock(newTable())).toEqual({
      kind: "table",
      fields: { header: true },
      blocks: Array.from({ length: 3 }, () => ({ kind: "row", fields: { cells: ["", "", ""] } })),
    });
  });
});

describe("reading Markdown tables", () => {
  it("splits a row at unescaped pipes, its outer pipes optional", () => {
    expect(splitRow("| a | b |")).toEqual(["a", "b"]);
    expect(splitRow("a | b")).toEqual(["a", "b"]);
    expect(splitRow("| a \\| b | `c\\|d` |")).toEqual(["a | b", "`c|d`"]);
    expect(splitRow("|  | x")).toEqual(["", "x"]);
  });

  it("reads the header, the alignments from the delimiter line and the body", () => {
    const t = tableIn("| Item | Qty | Notes | Plain |\n| :--- | ---: | :---: | --- |\n| Apples | 3 | ripe | x |\n| Pears | 10 | | y |\n");
    expect(t).toEqual({
      header: true,
      align: ["left", "right", "center", "left"],
      rows: [
        ["Item", "Qty", "Notes", "Plain"],
        ["Apples", "3", "ripe", "x"],
        ["Pears", "10", "", "y"],
      ],
    });
  });

  it("reads tables without outer pipes, and widens for a row longer than the header", () => {
    const t = tableIn("a | b\n-- | --\n1 | 2 | 3\n4 |");
    expect(t.rows).toEqual([
      ["a", "b", ""],
      ["1", "2", "3"],
      ["4", "", ""],
    ]);
    expect(t.align).toEqual(["left", "left", "left"]);
  });

  it("turns each cell's inline Markdown into inline markup, escaped pipes and all", () => {
    const t = tableIn(
      "| Markdown | More |\n|---|---|\n| **b** _i_ `c\\|d` ~~s~~ | [site](https://example.com) and [note](/notes/" + ID + ") |\n| one<br>two | 2 \\* 3 &amp; <u>u</u> [[" + ID + "]] |",
    );
    expect(t.rows[1]).toEqual(["**b** *i* `c|d` ~~s~~", `[site](https://example.com) and [[${ID}|note]]`]);
    expect(t.rows[2]).toEqual(["one\ntwo", `2 \\* 3 & ++u++ [[${ID}]]`]);
  });

  it("keeps the lines around a table, and leaves text that is not a table alone", () => {
    const pieces = markdownPieces("Before\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nAfter\n");
    expect(pieces?.map((p) => p.kind)).toEqual(["text", "table", "text"]);
    expect(pieces?.[0]).toEqual({ kind: "text", lines: ["Before", ""] });
    expect(pieces?.[2]).toEqual({ kind: "text", lines: ["", "After"] });
    for (const text of [
      "a | b\nc | d",
      "Just some text\n---\nwith a rule",
      "| a | b |\n|---|\n| 1 | 2 |",
      "| a | b |",
      "|---|---|\n|---|---|",
      "plain words",
    ]) {
      expect(markdownPieces(text), text).toBeNull();
    }
  });
});

describe("inline Markdown to inline markup", () => {
  it("converts emphasis, code, links and breaks, and escapes markup characters in text", () => {
    expect(markdownToMarkup("**bold** and *it* and __b__")).toBe("**bold** and *it* and **b**");
    expect(markdownToMarkup("***both***")).toBe("***both***");
    expect(markdownToMarkup("a {red} [x] `=`")).toBe("a \\{red} \\[x\\] `=`");
    expect(markdownToMarkup("<mark>hi</mark> <s>no</s>")).toBe("==hi== ~~no~~");
  });
});

describe("inline markup to Markdown", () => {
  it("writes the marks as Markdown, whitespace outside them", () => {
    expect(markupToMarkdown("**bold** *it* ~~gone~~ `code`")).toBe("**bold** *it* ~~gone~~ `code`");
    expect(markupToMarkdown("**bold **after")).toBe("**bold** after");
    expect(markupToMarkdown("++u++ ==h== {red}red{/}")).toBe("<u>u</u> <mark>h</mark> red");
    expect(markupToMarkdown("[a **b**](https://x.y/a_(b\\))")).toBe("[a **b**](https://x.y/a_%28b%29)");
    expect(markupToMarkdown("2 \\* 3 _x_ a|b")).toBe("2 \\* 3 \\_x\\_ a\\|b");
    expect(markupToMarkdown("`a`b`")).toBe("`a`b\\`");
    expect(markupToMarkdown("`a\\`b`")).toBe("``a`b``");
  });

  it("writes a mention as a link to the note, with its label or title, or as itself", () => {
    expect(markupToMarkdown(`[[${ID}|the plan]]`)).toBe(`[the plan](/notes/${ID})`);
    expect(markupToMarkdown(`[[${ID}]]`, { titleOf: () => "Groceries" })).toBe(`[Groceries](/notes/${ID})`);
    expect(markupToMarkdown(`[[${ID}]]`)).toBe(`[[${ID}]]`);
    expect(markupToMarkdown("[[+task|Fix the pump]]")).toBe("Fix the pump");
  });
});

describe("tables to Markdown", () => {
  it("writes the header, the alignments and the body, pipes and breaks escaped", () => {
    const md = tableToMarkdown({
      header: true,
      align: ["left", "center", "right"],
      rows: [
        ["Item", "Qty", "Notes"],
        ["a | b", "`x|y`", "one\ntwo"],
      ],
    });
    expect(md).toBe("| Item | Qty | Notes |\n| --- | :---: | ---: |\n| a \\| b | `x\\|y` | one<br>two |");
  });

  it("gives a table with no header row an empty one", () => {
    expect(tableToMarkdown({ header: false, align: ["left", "left"], rows: [["a", "b"]] })).toBe("|  |  |\n| --- | --- |\n| a | b |");
  });

  it("reads back as the same table", () => {
    const t: TableData = {
      header: true,
      align: ["left", "right", "center"],
      rows: [
        ["**Item**", "Qty", "See"],
        ["*ripe* ~~old~~ `a|b`", "3 \\* 4", `[[${ID}|the plan]] and [site](https://example.com)`],
        ["a | b", "one\ntwo", ""],
      ],
    };
    expect(tableIn(tableToMarkdown(t))).toEqual(t);
  });
});

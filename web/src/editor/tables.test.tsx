import { Editor } from "@tiptap/core";
import { NodeSelection, TextSelection } from "@tiptap/pm/state";
import { CellSelection } from "@tiptap/pm/tables";
import { afterEach, describe, expect, it } from "vitest";
import type { Block } from "../api";
import { blocksToDoc, docToBlocks } from "./doc";
import { editorExtensions } from "./extensions";
import { addRowAbove, deleteTableHere, markdownSlice, setColumnAlign, setHeaderRow, sliceText } from "./tables";
import { addColumnAfter, deleteColumn, deleteRow } from "@tiptap/pm/tables";

const ID = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const p = (text: string): Block => ({ kind: "paragraph", fields: { text } });
const row = (...cells: string[]): Block => ({ kind: "row", fields: { cells } });
const table = (fields: Block["fields"], ...rows: Block[]): Block => ({ kind: "table", ...(fields ? { fields } : {}), blocks: rows });

/** Press `key` in `e` as the keyboard does: through the editor's key handlers. */
function press(e: Editor, key: string, shiftKey = false) {
  const event = new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true });
  e.view.someProp("handleKeyDown", (f) => f(e.view, event));
}

// jsdom lays nothing out.
if (!Element.prototype.scrollIntoView) Element.prototype.scrollIntoView = () => {};
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

const editors: Editor[] = [];
afterEach(() =>
  editors.splice(0).forEach((e) => {
    e.view.dom.parentElement?.remove();
    e.destroy();
  }),
);

function make(blocks: Block[]): Editor {
  // In the page, as the tools' buttons take their clicks through it.
  const element = document.body.appendChild(document.createElement("div"));
  const editor = new Editor({ element, extensions: editorExtensions(), content: blocksToDoc(blocks) });
  editors.push(editor);
  return editor;
}

/** The position inside cell `c` of row `r` of the first table, at the start of its text. */
function cellPos(e: Editor, r: number, c: number): number {
  let at = -1;
  e.state.doc.descendants((node, pos) => {
    if (at >= 0) return false;
    if (node.type.name !== "table") return true;
    let rowAt = pos + 1;
    for (let i = 0; i < r; i++) rowAt += node.child(i).nodeSize;
    let cellAt = rowAt + 1;
    for (let j = 0; j < c; j++) cellAt += node.child(r).child(j).nodeSize;
    at = cellAt + 1;
    return false;
  });
  return at;
}

function caretIn(e: Editor, r: number, c: number) {
  e.view.dispatch(e.state.tr.setSelection(TextSelection.create(e.state.doc, cellPos(e, r, c))));
}

/** Which cell the caret is in, as `[row, column]`. */
function caretCell(e: Editor): [number, number] {
  const $f = e.state.selection.$from;
  for (let d = $f.depth; d > 0; d--) {
    if ($f.node(d).type.name === "tableRow") return [$f.index(d - 1), $f.index(d)];
  }
  return [-1, -1];
}

const blocksOf = (e: Editor) => docToBlocks(e.getJSON());

describe("tables in the editor document", () => {
  it("convert to editor nodes and back, header, alignments, markup and all", () => {
    const blocks: Block[] = [
      table(
        { header: true, align: ["left", "right", "center"] },
        row("Item", "Qty", "**Notes**"),
        row("Apples", "3", `see [[${ID}|the plan]] and *more*`),
        row("Pears", "line\nbreak", ""),
      ),
      table(undefined, row("plain", "row")),
    ];
    const doc = blocksToDoc(blocks);
    expect(doc.content!.map((n) => [n.type, n.content!.map((r) => r.content!.map((c) => c.type).join(","))])).toEqual([
      ["table", ["tableHeader,tableHeader,tableHeader", "tableCell,tableCell,tableCell", "tableCell,tableCell,tableCell"]],
      ["table", ["tableCell,tableCell"]],
    ]);
    expect(docToBlocks(doc)).toEqual(blocks);
    // Through a real editor, the schema accepts them.
    expect(blocksOf(make(blocks))).toEqual(blocks);
  });

  it("reads a short row padded to the widest", () => {
    const e = make([table(undefined, row("a", "b"), row("c"))]);
    expect(blocksOf(e)).toEqual([table(undefined, row("a", "b"), row("c", ""))]);
  });
});

describe("keys in a table", () => {
  it("Tab and Shift+Tab move between cells, and Tab in the last cell adds a row", () => {
    const e = make([table({ header: true }, row("a", "b"), row("c", "d"))]);
    caretIn(e, 0, 0);
    press(e, "Tab");
    expect(caretCell(e)).toEqual([0, 1]);
    press(e, "Tab");
    expect(caretCell(e)).toEqual([1, 0]);
    press(e, "Tab", true);
    expect(caretCell(e)).toEqual([0, 1]);
    caretIn(e, 1, 1);
    press(e, "Tab");
    expect(caretCell(e)).toEqual([2, 0]);
    e.commands.insertContent("new");
    expect(blocksOf(e)).toEqual([table({ header: true }, row("a", "b"), row("c", "d"), row("new", ""))]);
  });

  it("Enter moves to the cell below, and out of the table after the last row", () => {
    const e = make([table(undefined, row("a", "b"), row("c", "d"))]);
    caretIn(e, 0, 1);
    press(e, "Enter");
    expect(caretCell(e)).toEqual([1, 1]);
    press(e, "Enter");
    expect(caretCell(e)).toEqual([-1, -1]);
    expect(e.state.selection.$from.parent.type.name).toBe("paragraph");
    e.commands.insertContent("after");
    expect(blocksOf(e)).toEqual([table(undefined, row("a", "b"), row("c", "d")), p("after")]);
  });

  it("Shift+Enter is a line break inside the cell", () => {
    const e = make([table(undefined, row("ab"))]);
    e.view.dispatch(e.state.tr.setSelection(TextSelection.create(e.state.doc, cellPos(e, 0, 0) + 1)));
    press(e, "Enter", true);
    expect(blocksOf(e)).toEqual([table(undefined, row("a\nb"))]);
  });
});

describe("rows and columns", () => {
  const run = (e: Editor, cmd: (s: Editor["state"], d: Editor["view"]["dispatch"]) => boolean) => cmd(e.state, e.view.dispatch);

  it("adds and deletes rows and columns", () => {
    const e = make([table({ header: true }, row("a", "b"), row("c", "d"))]);
    caretIn(e, 1, 0);
    run(e, addColumnAfter);
    expect(blocksOf(e)).toEqual([table({ header: true }, row("a", "", "b"), row("c", "", "d"))]);
    caretIn(e, 1, 2);
    run(e, deleteColumn);
    caretIn(e, 1, 0);
    run(e, deleteRow);
    expect(blocksOf(e)).toEqual([table({ header: true }, row("a", ""))]);
  });

  it("keeps the header row first when a row is added above it", () => {
    const e = make([table({ header: true }, row("h1", "h2"), row("c", "d"))]);
    caretIn(e, 0, 1);
    run(e, addRowAbove);
    expect(blocksOf(e)).toEqual([table({ header: true }, row("", ""), row("h1", "h2"), row("c", "d"))]);
    caretIn(e, 2, 0);
    run(e, addRowAbove);
    expect(blocksOf(e)).toEqual([table({ header: true }, row("", ""), row("h1", "h2"), row("", ""), row("c", "d"))]);
  });

  it("turns the header row on and off, and aligns a column", () => {
    const e = make([table(undefined, row("a", "b"), row("c", "d"))]);
    caretIn(e, 1, 1);
    run(e, setHeaderRow(true));
    run(e, setColumnAlign("right"));
    expect(blocksOf(e)).toEqual([table({ header: true, align: ["left", "right"] }, row("a", "b"), row("c", "d"))]);
    run(e, setColumnAlign("left"));
    run(e, setHeaderRow(false));
    expect(blocksOf(e)).toEqual([table(undefined, row("a", "b"), row("c", "d"))]);
  });

  it("are on the table's tools, which act on the cell holding the caret", () => {
    const e = make([table({ header: true }, row("a", "b"), row("c", "d")), p("after")]);
    caretIn(e, 1, 1);
    const tools = e.view.dom.querySelector("[role=toolbar][aria-label=Table]")!;
    // Focus on the tools may take the page's selection out of the table: they act where it was.
    e.view.dispatch(e.state.tr.setSelection(TextSelection.create(e.state.doc, e.state.doc.content.size - 2)));
    const align = tools.querySelector<HTMLSelectElement>("select[aria-label='Column alignment']")!;
    align.value = "right";
    align.dispatchEvent(new Event("change", { bubbles: true }));
    expect(blocksOf(e)).toEqual([table({ header: true, align: ["left", "right"] }, row("a", "b"), row("c", "d")), p("after")]);
    expect(align.value).toBe("right");
    tools.querySelector<HTMLButtonElement>("button[aria-label='Add row below']")!.click();
    tools.querySelector<HTMLButtonElement>("button[aria-label='Header row']")!.click();
    expect(blocksOf(e)).toEqual([table({ align: ["left", "right"] }, row("a", "b"), row("c", "d"), row("", "")), p("after")]);
  });

  it("deletes the whole table", () => {
    const e = make([p("before"), table(undefined, row("a")), p("after")]);
    caretIn(e, 0, 0);
    run(e, deleteTableHere);
    expect(blocksOf(e)).toEqual([p("before"), p("after")]);
  });

  it("keeps header cells to the first row, and tables out of lists", () => {
    const e = make([table({ header: true }, row("a", "b"), row("c", "d"))]);
    // A header cell in a body row, as a pasted header column would make, becomes a plain one.
    const pos = cellPos(e, 1, 0) - 1;
    e.view.dispatch(e.state.tr.setNodeMarkup(pos, e.state.schema.nodes.tableHeader, e.state.doc.nodeAt(pos)!.attrs));
    expect(e.state.doc.nodeAt(pos)!.type.name).toBe("tableCell");

    const l = make([{ kind: "list", fields: { style: "bullet" }, blocks: [{ kind: "item", fields: { text: "one" } }] }]);
    const md = markdownSlice(l.state.schema, "| x |\n|---|\n| y |")!;
    l.view.dispatch(l.state.tr.setSelection(TextSelection.create(l.state.doc, 6)).replaceSelection(md));
    expect(blocksOf(l)).toEqual([
      { kind: "list", fields: { style: "bullet" }, blocks: [{ kind: "item", fields: { text: "one" } }] },
      table({ header: true }, row("x"), row("y")),
    ]);
  });
});

describe("the clipboard", () => {
  /** Paste `text` and `html` at the caret, as the browser hands them over. */
  function paste(e: Editor, text: string, html = "") {
    const clipboardData = { getData: (t: string) => (t === "text/plain" ? text : t === "text/html" ? html : ""), files: [], items: [], types: html ? ["text/html", "text/plain"] : ["text/plain"] };
    const event = Object.assign(new Event("paste", { cancelable: true }), { clipboardData }) as unknown as ClipboardEvent;
    if (!e.view.someProp("handlePaste", (f) => f(e.view, event, markdownSlice(e.state.schema, "x")!))) {
      if (html) e.view.pasteHTML(html, event);
      else e.view.pasteText(text, event);
    }
  }

  it("pastes a Markdown table as a table, the lines around it as paragraphs", () => {
    const e = make([p("")]);
    e.commands.setTextSelection(1);
    e.view.pasteText("Intro\n| Item | Qty |\n| :--- | ---: |\n| **Apples** | 3 |\nOutro", new Event("paste") as ClipboardEvent);
    expect(blocksOf(e)).toEqual([p("Intro"), table({ header: true, align: ["left", "right"] }, row("Item", "Qty"), row("**Apples**", "3")), p("Outro")]);
  });

  it("pastes a Markdown table from text that came with HTML of no table, such as a code editor's", () => {
    const e = make([p("")]);
    e.commands.setTextSelection(1);
    const md = "| a | b |\n|---|---|\n| `x\\|y` | 2 |";
    paste(e, md, `<div><span>${md}</span></div>`);
    expect(blocksOf(e)).toEqual([table({ header: true }, row("a", "b"), row("`x|y`", "2"))]);
  });

  it("pastes an HTML table, as a spreadsheet puts on the clipboard, as a table", () => {
    const e = make([p("")]);
    e.commands.setTextSelection(1);
    const html =
      '<google-sheets-html-origin><table><colgroup><col></colgroup><tbody><tr><td style="font-weight:bold">Name</td><td style="text-align:right">Qty</td></tr><tr><td><p>Apples <b>fresh</b></p></td><td style="text-align:right">3</td></tr></tbody></table></google-sheets-html-origin>';
    paste(e, "Name\tQty\nApples fresh\t3", html);
    // The spreadsheet's bold header cell is bold text.
    expect(blocksOf(e)).toEqual([table({ align: ["left", "right"] }, row("**Name**", "Qty"), row("Apples **fresh**", "3"))]);
  });

  it("leaves text that is not a table to paste as it always did", () => {
    const e = make([p("")]);
    e.commands.setTextSelection(1);
    e.view.pasteText("a | b\nc | d", new Event("paste") as ClipboardEvent);
    expect(blocksOf(e).map((b) => b.kind)).toEqual(["paragraph", "paragraph"]);
  });

  it("copies a table as a Markdown table and an HTML table", () => {
    const e = make([p("before"), table({ header: true, align: ["left", "center"] }, row("Item", "Qty"), row(`[[${ID}|plan]] a|b`, "**3**"))]);
    const at = e.state.doc.child(0).nodeSize;
    const slice = NodeSelection.create(e.state.doc, at).content();
    const md = "| Item | Qty |\n| --- | :---: |\n| [plan](/notes/" + ID + ") a\\|b | **3** |";
    expect(sliceText(slice)).toBe(md);
    const { dom, text } = e.view.serializeForClipboard(slice);
    expect(text).toBe(md);
    expect(dom.querySelector("table th")?.textContent).toBe("Item");
    expect(dom.querySelector("table td strong")?.textContent).toBe("3");
    expect(dom.querySelector("table td[style*=center]")).not.toBeNull();
    // Text with no table is TipTap's to write, as before.
    expect(sliceText(TextSelection.create(e.state.doc, 1, 4).content())).toBeUndefined();
  });

  it("copies a range of cells as a Markdown table of those cells", () => {
    const e = make([table({ header: true }, row("a", "b", "c"), row("d", "e", "f"), row("g", "h", "i"))]);
    const sel = CellSelection.create(e.state.doc, cellPos(e, 1, 1) - 1, cellPos(e, 2, 2) - 1);
    expect(sliceText(sel.content())).toBe("|  |  |\n| --- | --- |\n| e | f |\n| h | i |");
    const top = CellSelection.create(e.state.doc, cellPos(e, 0, 0) - 1, cellPos(e, 1, 1) - 1);
    expect(sliceText(top.content())).toBe("| a | b |\n| --- | --- |\n| d | e |");
  });
});

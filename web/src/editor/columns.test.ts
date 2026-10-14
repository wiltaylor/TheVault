import { Editor } from "@tiptap/core";
import { TextSelection } from "@tiptap/pm/state";
import { afterAll, describe, expect, it } from "vitest";
import type { Block, TemplateItem, TypeDef } from "../api";
import { addedWidth, newColumns, nudgePair, slotsOf, splitPair } from "../blocks/columns";
import { type TemplateModel, followFields, typeFromSchema, unplaced } from "../defs/typeModel";
import { TITLE_FIELD, unplacedFields } from "../notes";
import { blockOfSelection, edgeDropTr, moveBlockTr } from "./blockTools";
import { addColumnTr, equalWidthsTr, normalizeTr, removeColumnTr, setPairTr } from "./columns";
import { blocksToDoc, docToBlocks, docToItems, itemsToDoc } from "./doc";
import { editorExtensions } from "./extensions";

const p = (text: string): Block => ({ kind: "paragraph", fields: { text } });
const col = (width: number, ...blocks: Block[]): Block =>
  blocks.length ? { kind: "column", fields: { width }, blocks } : { kind: "column", fields: { width } };
const layout = (...columns: Block[]): Block => ({ kind: "columns", blocks: columns });

describe("columns in the editor document", () => {
  it("converts a layout to columns of editable blocks and back, widths and all", () => {
    const blocks: Block[] = [
      p("above"),
      layout(
        col(2, { kind: "heading", fields: { level: 2, text: "Left **bold**" } }, p("more")),
        col(0.5, { kind: "code", fields: { source: "x" } }),
        col(1),
      ),
    ];
    const doc = blocksToDoc(blocks);
    const cols = doc.content![1]!;
    expect(cols.type).toBe("columns");
    expect(cols.content!.map((c) => [c.type, c.attrs?.width, c.content!.map((n) => n.type)])).toEqual([
      ["column", 2, ["heading", "paragraph"]],
      ["column", 0.5, ["vaultBlock"]],
      // An empty column gets a line to type on.
      ["column", 1, ["paragraph"]],
    ]);
    expect(docToBlocks(doc)).toEqual(blocks);
  });

  it("drops the empty lines at the end of a column, as at the end of the document", () => {
    const doc = {
      type: "doc",
      content: [
        {
          type: "columns",
          content: [
            { type: "column", attrs: { width: 1 }, content: [{ type: "paragraph", content: [{ type: "text", text: "a" }] }, { type: "paragraph" }] },
            { type: "column", attrs: { width: 3 }, content: [{ type: "paragraph" }] },
          ],
        },
        { type: "paragraph" },
      ],
    };
    expect(docToBlocks(doc)).toEqual([layout(col(1, p("a")), col(3))]);
  });

  it("converts a template's columns of slots and fixed blocks both ways", () => {
    const items: TemplateItem[] = [
      {
        block: layout(
          col(1, { kind: "slot", labels: ["status"] }, { kind: "paragraph", fields: { text: "Due {{due}}" } }),
          col(1, { kind: "slot", labels: ["due"], fields: { label: "When" } }),
        ),
      },
      { slot: "body", label: null },
    ];
    const doc = itemsToDoc(items);
    const first = doc.content![0]!.content![0]!.content!;
    expect(first[0]).toEqual({ type: "templateSlot", attrs: { slot: "status", label: null } });
    expect(first[1]!.content!.some((n) => n.type === "fieldRef")).toBe(true);
    expect(doc.content![0]!.content![1]!.content![0]).toEqual({ type: "templateSlot", attrs: { slot: "due", label: "When" } });
    expect(docToItems(doc)).toEqual(items);
  });
});

describe("layout rules in the editor", () => {
  const editors: Editor[] = [];
  afterAll(() => editors.forEach((e) => e.destroy()));
  const make = (blocks: Block[]) => {
    const editor = new Editor({ extensions: editorExtensions(), content: blocksToDoc(blocks) });
    editors.push(editor);
    return editor;
  };
  const blocks = (e: Editor) => docToBlocks(e.getJSON());
  /** The position of the text `t` in the document. */
  const at = (e: Editor, t: string) => {
    let found = -1;
    e.state.doc.descendants((n, pos) => {
      if (found < 0 && n.isText && n.text?.includes(t)) found = pos + n.text.indexOf(t);
    });
    return found;
  };
  /** Delete every block of column `index` of the first layout. */
  const empty = (e: Editor, index: number) => {
    const doc = e.state.doc;
    let pos = 0;
    for (let i = 0; i < doc.childCount && doc.child(i).type.name !== "columns"; i++) pos += doc.child(i).nodeSize;
    const lay = doc.nodeAt(pos)!;
    let c = pos + 1;
    for (let i = 0; i < index; i++) c += lay.child(i).nodeSize;
    e.view.dispatch(e.state.tr.delete(c + 1, c + lay.child(index).nodeSize - 1));
  };

  it("removes a column left with no blocks, and unwraps a layout left with one column", () => {
    const e = make([p("top"), layout(col(1, p("a")), col(2, p("b")), col(1, p("c"))), p("end")]);
    empty(e, 1);
    expect(blocks(e)).toEqual([p("top"), layout(col(1, p("a")), col(1, p("c"))), p("end")]);
    // The caret stays in what is left: put it in "c", then empty the first column.
    e.view.dispatch(e.state.tr.setSelection(TextSelection.create(e.state.doc, at(e, "c") + 1)));
    empty(e, 0);
    expect(blocks(e)).toEqual([p("top"), p("c"), p("end")]);
    expect(e.state.selection.$from.parent.textContent).toBe("c");
  });

  it("removes a layout whose columns are all emptied at once", () => {
    const e = make([layout(col(1, p("a")), col(1, p("b")))]);
    const lay = e.state.doc.child(0);
    const tr = e.state.tr;
    // Last column first, so the first one's position holds.
    const second = 1 + lay.child(0).nodeSize;
    tr.delete(second + 1, second + lay.child(1).nodeSize - 1);
    tr.delete(2, 1 + lay.child(0).nodeSize - 1);
    e.view.dispatch(tr);
    expect(e.state.doc.child(0).type.name).toBe("paragraph");
    expect(blocks(e)).toEqual([]);
  });

  it("leaves well-formed layouts alone", () => {
    const e = make([layout(col(1, p("a")), col(1))]);
    expect(normalizeTr(e.state)).toBeNull();
  });

  it("removes a column, keeping its blocks in the one before, or the next for the first", () => {
    const e = make([layout(col(1, p("a")), col(1, p("b")), col(1, p("c")))]);
    e.view.dispatch(removeColumnTr(e.state, 0, 1)!);
    expect(blocks(e)).toEqual([layout(col(1, p("a"), p("b")), col(1, p("c")))]);
    e.view.dispatch(removeColumnTr(e.state, 0, 0)!);
    // One column left: the layout unwraps.
    expect(blocks(e)).toEqual([p("a"), p("b"), p("c")]);
  });

  it("adds a column of the mean width, up to six, and resets the widths", () => {
    const e = make([layout(col(1, p("a")), col(2, p("b")))]);
    e.view.dispatch(addColumnTr(e.state, 0)!);
    expect(blocks(e)).toEqual([layout(col(1, p("a")), col(2, p("b")), col(1.5))]);
    // The caret is in the new column.
    expect(e.state.selection.$from.node(2).attrs.width).toBe(1.5);
    for (let i = 0; i < 3; i++) e.view.dispatch(addColumnTr(e.state, 0)!);
    expect(e.state.doc.child(0).childCount).toBe(6);
    expect(addColumnTr(e.state, 0)).toBeNull();
    e.view.dispatch(equalWidthsTr(e.state, 0)!);
    e.state.doc.child(0).forEach((c) => expect(c.attrs.width).toBe(1));
  });

  it("stores new widths for the two columns either side of a gap", () => {
    const e = make([layout(col(1, p("a")), col(1, p("b")), col(1, p("c")))]);
    e.view.dispatch(setPairTr(e.state, 0, 2, splitPair(1, 1, 0.25))!);
    expect(blocks(e)).toEqual([layout(col(1, p("a")), col(0.5, p("b")), col(1.5, p("c")))]);
    expect(setPairTr(e.state, 0, 0, [1, 1])).toBeNull();
  });

  it("moves blocks within a column with Alt+Arrow, and not out of it", () => {
    const e = make([p("top"), layout(col(1, p("a1"), p("a2")), col(1, p("b")))]);
    e.view.dispatch(e.state.tr.setSelection(TextSelection.create(e.state.doc, at(e, "a2") + 1)));
    const here = blockOfSelection(e.state)!;
    expect(here.parent.type.name).toBe("column");
    e.view.dispatch(moveBlockTr(e.state, here.index, -1, here)!);
    expect(blocks(e)).toEqual([p("top"), layout(col(1, p("a2"), p("a1")), col(1, p("b")))]);
    const again = blockOfSelection(e.state)!;
    expect(again.index).toBe(0);
    expect(moveBlockTr(e.state, again.index, -1, again)).toBeNull();
  });

  it("drops a block beside a top-level block as two columns, and beside a column's block as a new column", () => {
    const e = make([p("one"), p("two"), p("three")]);
    const two = e.state.doc.child(0).nodeSize;
    const three = two + e.state.doc.child(1).nodeSize;
    // "three" dropped on the right edge of "one".
    e.view.dispatch(edgeDropTr(e.state, { pos: three, node: e.state.doc.child(2) }, { pos: 0, side: "right" })!);
    expect(blocks(e)).toEqual([layout(col(1, p("one")), col(1, p("three"))), p("two")]);

    // "two" dropped left of "three", inside the layout: a new column there, sharing its width.
    const lay = e.state.doc.child(0);
    const threePos = 1 + lay.child(0).nodeSize + 1;
    e.view.dispatch(edgeDropTr(e.state, { pos: lay.nodeSize, node: e.state.doc.child(1) }, { pos: threePos, side: "left" })!);
    expect(blocks(e)).toEqual([layout(col(1, p("one")), col(0.5, p("two")), col(0.5, p("three")))]);
  });

  it("dragging the only block out of a column removes the column", () => {
    const e = make([layout(col(1, p("a")), col(1, p("b"))), p("after")]);
    const lay = e.state.doc.child(0);
    const bPos = 1 + lay.child(0).nodeSize + 1;
    // "b" dropped on the right edge of "after": "a" unwraps, "after" and "b" make a new layout.
    e.view.dispatch(edgeDropTr(e.state, { pos: bPos, node: e.state.doc.nodeAt(bPos)! }, { pos: lay.nodeSize, side: "right" })!);
    expect(blocks(e)).toEqual([p("a"), layout(col(1, p("after")), col(1, p("b")))]);
  });
});

describe("column widths", () => {
  it("splits a pair by the gap's position, keeping their sum and a tenth each at least", () => {
    expect(splitPair(1, 1, 0.5)).toEqual([1, 1]);
    expect(splitPair(1, 2, 0.5)).toEqual([1.5, 1.5]);
    expect(splitPair(1, 1, 0)).toEqual([0.2, 1.8]);
    expect(splitPair(1, 1, 1)).toEqual([1.8, 0.2]);
    expect(nudgePair(1, 1, 0.05)).toEqual([1.1, 0.9]);
    expect(nudgePair(1, 1, -0.05)).toEqual([0.9, 1.1]);
    expect(addedWidth([1, 2])).toBe(1.5);
    expect(newColumns(9).blocks).toHaveLength(6);
    expect(newColumns(3).blocks).toEqual([col(1), col(1), col(1)]);
  });
});

describe("slots in columns", () => {
  const items: TemplateItem[] = [
    { slot: "title", label: null },
    { block: layout(col(1, { kind: "slot", labels: ["status"] }), col(1, p("x"), { kind: "slot", labels: ["due"], fields: { label: "When" } })) },
    { block: p("{{owner}}") },
  ];

  it("count as placed for the properties panel and the template's / menu", () => {
    expect(slotsOf(items)).toEqual(["title", "status", "due"]);
    const type: TypeDef = {
      name: "t",
      label: "T",
      plural: "Ts",
      description: null,
      icon: null,
      colour: null,
      fields: ["status", "due", "owner"].map((name) => ({ ...TITLE_FIELD, name, label: name, required: false })),
      templates: [{ name: "default", label: null, tabs: [{ title: "Main", items }] }],
    };
    expect(unplacedFields(type).map((f) => f.name)).toEqual(["owner"]);
    const t: TemplateModel = { name: "default", label: null, tabs: [{ title: "Main", items }] };
    expect(unplaced(t, type.fields)).toEqual(["owner"]);
  });

  it("follow renamed and removed fields", () => {
    const m = typeFromSchema({
      name: "t",
      label: "T",
      fields: [
        { name: "status", kind: "string" },
        { name: "due", kind: "date" },
      ],
      templates: [{ name: "default", label: null, tabs: [{ title: "Main", items }] }],
    } as never);
    m.fields[0]!.name = "state";
    m.fields.splice(1, 1);
    const block = (followFields(m)[0]!.tabs[0]!.items[1] as { block: Block }).block;
    expect(block).toEqual(layout(col(1, { kind: "slot", labels: ["state"] }), col(1, p("x"))));
  });
});

import { Editor } from "@tiptap/core";
import { NodeSelection, TextSelection } from "@tiptap/pm/state";
import { afterAll, describe, expect, it } from "vitest";
import { matchFieldRef, matchSlash, matchesQuery, moveBlockTr } from "./blockTools";
import { blocksToDoc, docToBlocks } from "./doc";
import { editorExtensions } from "./extensions";

describe("block menu triggers", () => {
  it("opens on / at the start of an empty paragraph or after a space", () => {
    expect(matchSlash("/", "")).toEqual({ length: 1, query: "" });
    expect(matchSlash("/head", "")).toEqual({ length: 5, query: "head" });
    expect(matchSlash("some text /co", "")).toEqual({ length: 3, query: "co" });
    expect(matchSlash("some text /co", " more")).toEqual({ length: 3, query: "co" });
  });

  it("stays shut inside words, paths and non-empty paragraphs", () => {
    expect(matchSlash("and/or", "")).toBeNull();
    expect(matchSlash("/usr/bin", "")).toBeNull();
    expect(matchSlash("/", "text after")).toBeNull();
    expect(matchSlash("/a b", "")).toBeNull();
  });

  it("opens the field menu on {{", () => {
    expect(matchFieldRef("Due {{")).toEqual({ length: 2, query: "" });
    expect(matchFieldRef("Due {{sta")).toEqual({ length: 5, query: "sta" });
    expect(matchFieldRef("Due {{sta ")).toBeNull();
  });

  it("filters by every word of the query", () => {
    expect(matchesQuery("Heading 2 Text h2", "head 2")).toBe(true);
    expect(matchesQuery("Numbered list", "bul")).toBe(false);
    expect(matchesQuery("Anything", "")).toBe(true);
    // Word starts, not anywhere inside: "ink" is not in "link".
    expect(matchesQuery("Mention a note link", "ink")).toBe(false);
    expect(matchesQuery("Ink drawing", "ink")).toBe(true);
    expect(matchesQuery("Estimate estimate", "esti")).toBe(true);
  });
});

describe("moving blocks", () => {
  const editors: Editor[] = [];
  afterAll(() => editors.forEach((e) => e.destroy()));
  const make = () => {
    const editor = new Editor({
      extensions: editorExtensions(),
      content: blocksToDoc([
        { kind: "paragraph", fields: { text: "one" } },
        { kind: "code", fields: { source: "x" } },
        { kind: "paragraph", fields: { text: "three" } },
      ]),
    });
    editors.push(editor);
    return editor;
  };
  const order = (e: Editor) => docToBlocks(e.getJSON()).map((b) => String(b.fields?.text ?? b.kind));

  it("moves the block holding the caret, and keeps the caret in it", () => {
    const e = make();
    e.view.dispatch(e.state.tr.setSelection(TextSelection.create(e.state.doc, 2)));
    e.view.dispatch(moveBlockTr(e.state, 0, 1)!);
    expect(order(e)).toEqual(["code", "one", "three"]);
    expect(e.state.selection.$from.parent.textContent).toBe("one");
    expect(moveBlockTr(e.state, 0, -1)).toBeNull();
  });

  it("moves a selected block node", () => {
    const e = make();
    const codePos = e.state.doc.child(0).nodeSize;
    e.view.dispatch(e.state.tr.setSelection(NodeSelection.create(e.state.doc, codePos)));
    e.view.dispatch(moveBlockTr(e.state, 1, 1)!);
    expect(order(e)).toEqual(["one", "three", "code"]);
    expect((e.state.selection as NodeSelection).node.type.name).toBe("vaultBlock");
  });
});

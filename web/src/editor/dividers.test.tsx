import { Editor } from "@tiptap/core";
import { NodeSelection } from "@tiptap/pm/state";
import { afterEach, describe, expect, it } from "vitest";
import { render } from "@solidjs/testing-library";
import type { Block } from "../api";
import { Blocks } from "../components/Render";
import { blocksToDoc, docToBlocks } from "./doc";
import { editorExtensions } from "./extensions";
import { markdownSlice, sliceText } from "./tables";

// rich-import H5: the `divider` block in the editor, on the clipboard and in read-only views.

const p = (text: string): Block => ({ kind: "paragraph", fields: { text } });
const divider: Block = { kind: "divider" };

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
  const element = document.body.appendChild(document.createElement("div"));
  const editor = new Editor({ element, extensions: editorExtensions(), content: blocksToDoc(blocks) });
  editors.push(editor);
  return editor;
}

const blocksOf = (e: Editor) => docToBlocks(e.getJSON());

/** Type `text` at the caret as the keyboard does, through the input rules. */
function type(e: Editor, text: string) {
  for (const ch of text) {
    const { from, to } = e.state.selection;
    const handled = e.view.someProp("handleTextInput", (f) => f(e.view, from, to, ch, () => e.state.tr.insertText(ch, from, to)));
    if (!handled) e.view.dispatch(e.state.tr.insertText(ch, from, to));
  }
}

describe("the divider block", () => {
  it("round-trips through the editor's document", () => {
    const blocks = [p("above"), divider, p("below")];
    const e = make(blocks);
    expect(e.state.doc.child(1).type.name).toBe("horizontalRule");
    expect(blocksOf(e)).toEqual(blocks);
  });

  it("is what typing --- on an empty line makes", () => {
    const e = make([p("above"), p("")]);
    e.commands.setTextSelection(e.state.doc.child(0).nodeSize + 1);
    type(e, "---");
    expect(blocksOf(e).slice(0, 2)).toEqual([p("above"), divider]);
  });

  it("pastes from Markdown thematic breaks, the lines around them as paragraphs", () => {
    const e = make([p("")]);
    const slice = markdownSlice(e.state.schema, "one\n---\ntwo\n***\n___")!;
    expect(slice).not.toBeNull();
    e.view.dispatch(e.state.tr.replaceSelection(slice));
    expect(blocksOf(e).filter((b) => !(b.kind === "paragraph" && b.fields?.text === ""))).toEqual([p("one"), divider, p("two"), divider, divider]);
  });

  it("pastes from an HTML <hr>", () => {
    const e = make([p("")]);
    e.commands.setTextSelection(1);
    e.view.pasteHTML("<p>one</p><hr><p>two</p>", new Event("paste") as ClipboardEvent);
    expect(blocksOf(e).map((b) => b.kind)).toEqual(["paragraph", "divider", "paragraph"]);
  });

  it("copies as a Markdown ---", () => {
    const e = make([p("above"), divider, p("below")]);
    const slice = NodeSelection.create(e.state.doc, e.state.doc.child(0).nodeSize).content();
    expect(sliceText(slice)).toBe("---");
  });

  it("renders as a rule in read-only views", () => {
    const { container } = render(() => <Blocks blocks={[p("a"), divider]} />);
    expect(container.querySelector("hr.divider-block")).not.toBeNull();
  });
});

import { Editor } from "@tiptap/core";
import { TextSelection } from "@tiptap/pm/state";
import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { Block } from "../api";
import { setFindText } from "../blocks/toggle";
import { ContentEditor, type EditorHandle } from "./ContentEditor";
import { blocksToDoc, docToBlocks } from "./doc";
import { editorExtensions } from "./extensions";
import { blockPath, nestTr, unnestTr, unwrapTr } from "./toggles";

const p = (text: string): Block => ({ kind: "paragraph", fields: { text } });

/** Press `key` in `e` as the keyboard does: through the editor's key handlers. */
function press(e: Editor, key: string, shiftKey = false) {
  const event = new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true });
  e.view.someProp("handleKeyDown", (f) => f(e.view, event));
}

// jsdom lays nothing out; the block menu places itself by the caret's rectangle.
if (!Element.prototype.getClientRects) Element.prototype.getClientRects = () => [] as unknown as DOMRectList;
if (!Element.prototype.scrollIntoView) Element.prototype.scrollIntoView = () => {};
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

beforeEach(() => {
  localStorage.clear();
  setFindText("", null);
});

describe("toggles and admonitions in the editor document", () => {
  it("convert to editor nodes and back, items and nested blocks and all", () => {
    const blocks: Block[] = [
      { kind: "toggle", fields: { level: 2, text: "Sum **bold**" }, blocks: [p("inside"), { kind: "code", fields: { source: "x" } }] },
      { kind: "toggle", fields: { text: "" } },
      {
        kind: "admonition",
        fields: { kind: "warning", title: "Mind **this**", colour: "pink", icon: "flame", collapsible: true },
        blocks: [{ kind: "toggle", fields: { text: "nested" }, blocks: [p("deep")] }],
      },
      { kind: "admonition", fields: { kind: "tip" } },
    ];
    const doc = blocksToDoc(blocks);
    expect(doc.content!.map((n) => [n.type, n.content!.map((c) => c.type)])).toEqual([
      ["toggle", ["toggleSummary", "paragraph", "vaultBlock"]],
      ["toggle", ["toggleSummary"]],
      ["admonition", ["admonitionTitle", "toggle"]],
      // An admonition always has a line to type on.
      ["admonition", ["admonitionTitle", "paragraph"]],
    ]);
    expect(doc.content![0]!.content![0]!.attrs).toEqual({ level: 2 });
    expect(docToBlocks(doc)).toEqual(blocks);
    // Through a real editor, the schema accepts every one of them.
    const editor = new Editor({ extensions: editorExtensions(), content: doc });
    expect(docToBlocks(editor.getJSON())).toEqual(blocks);
    editor.destroy();
  });

  it("keys a block's open state by its place in the content, the heading line not counted", () => {
    const editor = new Editor({
      extensions: editorExtensions(),
      content: blocksToDoc([p("a"), { kind: "toggle", fields: { text: "t" }, blocks: [p("b"), { kind: "toggle", fields: { text: "u" } }] }]),
    });
    const outer = editor.state.doc.child(0).nodeSize;
    expect(blockPath(editor.state.doc, outer)).toBe("1");
    const toggle = editor.state.doc.child(1);
    const inner = outer + 1 + toggle.child(0).nodeSize + toggle.child(1).nodeSize;
    expect(editor.state.doc.nodeAt(inner)?.type.name).toBe("toggle");
    expect(blockPath(editor.state.doc, inner)).toBe("1.1");
    editor.destroy();
  });
});

describe("nesting with Tab and Shift+Tab", () => {
  const editors: Editor[] = [];
  afterAll(() => editors.forEach((e) => e.destroy()));
  const make = (blocks: Block[]) => {
    const editor = new Editor({ element: document.createElement("div"), extensions: editorExtensions(), content: blocksToDoc(blocks) });
    editors.push(editor);
    return editor;
  };
  /** Put the caret in the first textblock whose text is `text`. */
  const caretIn = (e: Editor, text: string) => {
    let at = -1;
    e.state.doc.descendants((n, pos) => {
      if (at < 0 && n.isTextblock && n.textContent === text) at = pos + 1;
      return at < 0;
    });
    e.view.dispatch(e.state.tr.setSelection(TextSelection.create(e.state.doc, at)));
  };
  const blocks = (e: Editor) => docToBlocks(e.getJSON());

  it("Tab moves the block into the toggle or admonition above it, at its end, and Shift+Tab moves it out", () => {
    const e = make([{ kind: "toggle", fields: { text: "t" }, blocks: [p("in")] }, p("next"), p("after")]);
    caretIn(e, "next");
    e.view.dispatch(nestTr(e.state)!);
    expect(blocks(e)).toEqual([{ kind: "toggle", fields: { text: "t" }, blocks: [p("in"), p("next")] }, p("after")]);
    expect(e.state.selection.$from.parent.textContent).toBe("next");
    // Nothing above it in the toggle to go into: Tab does nothing.
    caretIn(e, "in");
    expect(nestTr(e.state)).toBeNull();
    caretIn(e, "in");
    e.view.dispatch(unnestTr(e.state)!);
    expect(blocks(e)).toEqual([{ kind: "toggle", fields: { text: "t" }, blocks: [p("next")] }, p("in"), p("after")]);
    expect(e.state.selection.$from.parent.textContent).toBe("in");
    // At the top level, Shift+Tab has nowhere to go.
    caretIn(e, "after");
    expect(unnestTr(e.state)).toBeNull();
  });

  it("works through the keys, into an admonition too, and leaves list items to the list", () => {
    const e = make([{ kind: "admonition", fields: { kind: "tip" }, blocks: [p("one")] }, p("two")]);
    caretIn(e, "two");
    press(e, "Tab");
    expect(blocks(e)).toEqual([{ kind: "admonition", fields: { kind: "tip" }, blocks: [p("one"), p("two")] }]);
    caretIn(e, "one");
    press(e, "Tab", true);
    // `one` leaves; the admonition keeps `two`.
    expect(blocks(e)).toEqual([{ kind: "admonition", fields: { kind: "tip" }, blocks: [p("two")] }, p("one")]);
    const l = make([
      { kind: "toggle", fields: { text: "t" } },
      { kind: "list", fields: { style: "bullet" }, blocks: [{ kind: "item", fields: { text: "a" } }, { kind: "item", fields: { text: "b" } }] },
    ]);
    caretIn(l, "b");
    expect(nestTr(l.state)).toBeNull();
  });

  it("Backspace at the start of a summary turns the toggle into a paragraph, its blocks kept", () => {
    const e = make([{ kind: "toggle", fields: { text: "" }, blocks: [p("kept")] }]);
    e.view.dispatch(e.state.tr.setSelection(TextSelection.create(e.state.doc, 2)));
    e.view.dispatch(unwrapTr(e.state)!);
    expect(blocks(e)).toEqual([p(""), p("kept")]);
  });

  it("`>> ` at the start of a paragraph makes a toggle, and `> ` stays the quote", () => {
    const e = make([p("")]);
    caretIn(e, "");
    const type = (text: string) => {
      for (const ch of text) {
        const { from, to } = e.state.selection;
        const handled = e.view.someProp("handleTextInput", (f) => f(e.view, from, to, ch, () => e.state.tr.insertText(ch, from, to)));
        if (!handled) e.view.dispatch(e.state.tr.insertText(ch, from, to));
      }
    };
    type(">> Later");
    expect(blocks(e)).toEqual([{ kind: "toggle", fields: { text: "Later" } }]);
    const q = make([p("")]);
    q.view.dispatch(q.state.tr.setSelection(TextSelection.create(q.state.doc, 1)));
    for (const ch of "> said") {
      const { from, to } = q.state.selection;
      const handled = q.view.someProp("handleTextInput", (f) => f(q.view, from, to, ch, () => q.state.tr.insertText(ch, from, to)));
      if (!handled) q.view.dispatch(q.state.tr.insertText(ch, from, to));
    }
    expect(docToBlocks(q.getJSON())).toEqual([{ kind: "quote", fields: { text: "said" } }]);
  });
});

describe("toggles in the content editor", () => {
  function mount(blocks: Block[], scope = "N/body") {
    const onChange = vi.fn();
    let handle!: EditorHandle;
    const r = render(() => <ContentEditor blocks={blocks} label="Body" onChange={onChange} toggleScope={scope} onReady={(h) => (handle = h)} />);
    return { ...r, onChange, editor: () => handle.editor };
  }
  const shown = (el: HTMLElement) => el.dataset.open !== "false";

  it("a toggle starts closed, opens and closes, and remembers it across a reload without saving the note", () => {
    const blocks: Block[] = [{ kind: "toggle", fields: { text: "Sum" }, blocks: [p("inside")] }];
    const first = mount(blocks);
    const box = first.getByTestId("toggle");
    expect(shown(box)).toBe(false);
    fireEvent.click(first.getByTestId("toggle-button"));
    expect(shown(box)).toBe(true);
    fireEvent.click(first.getByTestId("toggle-button"));
    expect(shown(box)).toBe(false);
    fireEvent.click(first.getByTestId("toggle-button"));
    // Opening is not an edit.
    expect(first.onChange).not.toHaveBeenCalled();
    first.unmount();
    const again = mount(blocks);
    expect(shown(again.getByTestId("toggle"))).toBe(true);
    again.unmount();
  });

  it("opens a toggle for a search hit inside it", () => {
    setFindText("N", "inside");
    const r = mount([
      { kind: "toggle", fields: { text: "a" }, blocks: [p("nothing here")] },
      { kind: "toggle", fields: { text: "b" }, blocks: [p("hit inside")] },
    ]);
    const [miss, hit] = r.getAllByTestId("toggle");
    expect(shown(miss!)).toBe(false);
    expect(shown(hit!)).toBe(true);
    r.unmount();
  });

  it("a collapsible admonition toggles, starting open; Enter at the end of an open summary starts its blocks", () => {
    const r = mount([
      { kind: "admonition", fields: { kind: "danger", collapsible: true }, blocks: [p("careful")] },
      { kind: "toggle", fields: { text: "Sum" } },
    ]);
    const box = r.getByTestId("admonition");
    expect(box.dataset.colour).toBe("red");
    expect(box.dataset.icon).toBe("octagon-alert");
    expect(shown(box)).toBe(true);
    const [fold, toggleButton] = r.getAllByTestId("toggle-button");
    fireEvent.click(fold!);
    expect(shown(box)).toBe(false);
    fireEvent.click(toggleButton!);
    const e = r.editor();
    let end = 0;
    e.state.doc.descendants((n, pos) => {
      if (n.type.name === "toggleSummary") end = pos + 1 + n.content.size;
    });
    e.view.dispatch(e.state.tr.setSelection(TextSelection.create(e.state.doc, end)));
    press(e, "Enter");
    expect(e.state.selection.$from.parent.type.name).toBe("paragraph");
    expect(e.state.selection.$from.node(-1).type.name).toBe("toggle");
    r.unmount();
  });

  it("the block menu inserts a toggle and a callout of the kind chosen", async () => {
    const r = mount([]);
    const e = r.editor();
    const slash = async (query: string, ...picks: string[]) => {
      e.view.focus();
      e.commands.insertContent(`/${query}`);
      await waitFor(() => expect(document.querySelector('[role="listbox"], .suggest-menu')).not.toBeNull());
      for (const label of picks) {
        const item = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find((o) => o.textContent?.includes(label));
        expect(item, label).toBeDefined();
        fireEvent.mouseDown(item!);
        fireEvent.click(item!);
      }
    };
    await slash("toggle", "Toggle heading 2");
    await waitFor(() => expect(docToBlocks(e.getJSON())).toEqual([{ kind: "toggle", fields: { level: 2, text: "" } }]));
    // The caret is in the summary: type it.
    e.commands.insertContent("Title");
    e.commands.setTextSelection(e.state.doc.content.size - 1);
    await slash("callout", "Callout", "Warning");
    await waitFor(() =>
      expect(docToBlocks(e.getJSON())).toEqual([
        { kind: "toggle", fields: { level: 2, text: "Title" } },
        { kind: "admonition", fields: { kind: "warning" } },
      ]),
    );
    r.unmount();
  });
});

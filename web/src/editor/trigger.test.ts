import { Editor } from "@tiptap/core";
import { afterAll, describe, expect, it } from "vitest";
import { editorExtensions } from "./extensions";
import { atOpensPicker } from "./trigger";

describe("@ opens the mention picker", () => {
  it("only at the start of a word", () => {
    for (const before of ["", "see ", "line\n", "(", "[", '"', "“", "'", "a, "]) {
      expect(atOpensPicker(before), JSON.stringify(before)).toBe(true);
    }
    for (const before of ["a", "mail me at wil", "x@y", "1", ".", "@", "-"]) {
      expect(atOpensPicker(before), JSON.stringify(before)).toBe(false);
    }
  });

  describe("in the editor", () => {
    const editor = new Editor({ element: document.createElement("div"), extensions: editorExtensions() });
    afterAll(() => editor.destroy());

    /** Type `text` one character at a time, as the keyboard would. */
    const type = (text: string) => {
      for (const ch of text) {
        const { from, to } = editor.state.selection;
        const handled = editor.view.someProp("handleTextInput", (f) => f(editor.view, from, to, ch, () => editor.state.tr));
        if (!handled) editor.view.dispatch(editor.state.tr.insertText(ch, from, to));
      }
    };
    const kinds = () => {
      const out: string[] = [];
      editor.state.doc.descendants((n) => {
        if (n.isInline) out.push(n.isText ? `text:${n.text}` : `${n.type.name}:${String(n.attrs.trigger ?? "")}`);
      });
      return out;
    };
    const fresh = () => {
      editor.commands.setContent({ type: "doc", content: [{ type: "paragraph" }] });
      editor.commands.focus("end");
    };

    it("turns @ at the start of a paragraph or after a space into the picker", () => {
      fresh();
      type("@");
      expect(kinds()).toEqual(["mentionPicker:@"]);
      fresh();
      type("ask @");
      expect(kinds()).toEqual(["text:ask ", "mentionPicker:@"]);
    });

    it("leaves @ inside a word as text", () => {
      fresh();
      type("wil@example.com");
      expect(kinds()).toEqual(["text:wil@example.com"]);
    });

    it("keeps [[ opening the picker", () => {
      fresh();
      type("see [[");
      expect(kinds()).toEqual(["text:see ", "mentionPicker:[["]);
    });
  });
});

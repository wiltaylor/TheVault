import { Editor } from "@tiptap/core";
import { afterEach, describe, expect, it } from "vitest";
import type { Block } from "../api";
import { blocksToDoc, docToBlocks } from "./doc";
import { editorExtensions } from "./extensions";
import { dataUrlFile, htmlWithPlaceholders, imageFiles, pastesFiles, placeholdersTr } from "./pasteImages";

const p = (text: string): Block => ({ kind: "paragraph", fields: { text } });
const png = (name = "shot.png") => new File([new Uint8Array([137, 80, 78, 71])], name, { type: "image/png" });

/** A promise with its resolve and reject at hand. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

let editors: Editor[] = [];
afterEach(() => {
  for (const e of editors) e.destroy();
  editors = [];
});

/** An editor on `blocks` whose uploads wait in `calls` until a test settles them. */
function setup(blocks: Block[]) {
  const calls: { file: File; done: ReturnType<typeof deferred<string>> }[] = [];
  const errors: string[] = [];
  const editor = new Editor({
    content: blocksToDoc(blocks),
    extensions: editorExtensions({
      images: {
        upload: (file) => {
          const done = deferred<string>();
          calls.push({ file, done });
          return done.promise;
        },
        onError: (file, e) => errors.push(`${file.name}: ${String(e)}`),
      },
    }),
  });
  editors.push(editor);
  return { editor, calls, errors };
}

/** The position `n` characters into the textblock holding `text`. */
function posIn(editor: Editor, text: string, n: number): number {
  let at = -1;
  editor.state.doc.descendants((node, pos) => {
    if (node.isTextblock && node.textContent === text) at = pos + 1 + n;
  });
  if (at < 0) throw new Error(`no ${text}`);
  return at;
}

const kinds = (editor: Editor) => editor.getJSON().content!.map((n) => (n.type === "vaultBlock" ? n.attrs!.block.kind : n.type));
const flush = () => new Promise((r) => setTimeout(r, 0));

describe("pasted and dropped images", () => {
  it("keeps image files, naming one without an image extension after its type", () => {
    const files = imageFiles([png("image"), new File(["x"], "a.txt", { type: "text/plain" }), png("b.PNG"), new File(["x"], "c", { type: "image/jpeg" })]);
    expect(files.map((f) => f.name)).toEqual(["image.png", "b.PNG", "c.jpg"]);
  });

  it("pastes a clipboard's image files when it has no text or its HTML is an image, else its text", () => {
    expect(pastesFiles("", "")).toBe(true);
    expect(pastesFiles(`<meta charset="utf-8"><img src="https://example.com/a.png">`, "")).toBe(true);
    expect(pastesFiles(`<img\nsrc="x">`, "https://example.com/a.png")).toBe(true);
    // An office suite's copied text, with a picture of it beside.
    expect(pastesFiles("<p>Some <b>text</b></p>", "Some text")).toBe(false);
    expect(pastesFiles("", "Some text")).toBe(false);
  });

  it("reads the file in an image data URL, and nothing else", () => {
    const f = dataUrlFile("data:image/png;base64,iVBORw==")!;
    expect(f.name).toBe("pasted-image.png");
    expect(f.type).toBe("image/png");
    expect(f.size).toBe(4);
    expect(dataUrlFile("data:image/svg+xml,%3Csvg%2F%3E")!.name).toBe("pasted-image.svg");
    expect(dataUrlFile("data:text/html,hi")).toBeNull();
    expect(dataUrlFile("https://example.com/a.png")).toBeNull();
  });

  it("turns data URL images in pasted HTML into placeholders and leaves other images alone", () => {
    const html = `<p>hi</p><img src="data:image/png;base64,iVBORw=="><img src="https://example.com/x.png">`;
    const out = htmlWithPlaceholders(html);
    expect(out).toMatch(/<div data-image-upload="upload-[^"]+"><\/div>/);
    expect(out).toContain(`<img src="https://example.com/x.png">`);
    expect(out).not.toContain("data:");
    expect(htmlWithPlaceholders("<p>plain</p>")).toBe("<p>plain</p>");
  });

  it("splits a paragraph at the caret, uploads, and puts the image there though typing went on", async () => {
    const { editor, calls } = setup([p("beforeafter"), p("last")]);
    const at = posIn(editor, "beforeafter", 6);
    editor.view.dispatch(placeholdersTr(editor.state, at, at, [png()]));
    expect(kinds(editor)).toEqual(["paragraph", "imageUpload", "paragraph", "paragraph"]);
    // Nothing is saved for the placeholder while it uploads.
    expect(docToBlocks(editor.getJSON())).toEqual([p("before"), p("after"), p("last")]);
    expect(calls).toHaveLength(1);

    // The caret is after the placeholder; type there, then in front of everything.
    editor.view.dispatch(editor.state.tr.insertText("X"));
    editor.view.dispatch(editor.state.tr.insertText("Y", 1));
    calls[0]!.done.resolve("01ARZ3NDEKTSV4RRFFQ69G5FAV");
    await flush();
    expect(docToBlocks(editor.getJSON())).toEqual([
      p("Ybefore"),
      { kind: "image", fields: { asset: "01ARZ3NDEKTSV4RRFFQ69G5FAV" } },
      p("Xafter"),
      p("last"),
    ]);
  });

  it("puts several images in order, whichever upload ends first", async () => {
    const { editor, calls } = setup([p("one")]);
    const at = posIn(editor, "one", 3);
    editor.view.dispatch(placeholdersTr(editor.state, at, at, [png("a.png"), png("b.png")]));
    expect(calls.map((c) => c.file.name)).toEqual(["a.png", "b.png"]);
    calls[1]!.done.resolve("B");
    await flush();
    calls[0]!.done.resolve("A");
    await flush();
    // At the end of the document, a line to type on follows the images.
    expect(kinds(editor)).toEqual(["paragraph", "image", "image", "paragraph"]);
    expect(docToBlocks(editor.getJSON())).toEqual([p("one"), { kind: "image", fields: { asset: "A" } }, { kind: "image", fields: { asset: "B" } }]);
  });

  it("replaces the selection, and goes after a list rather than inside it", () => {
    const list: Block = { kind: "list", fields: { style: "bullet" }, blocks: [{ kind: "item", fields: { text: "item" } }] };
    const { editor } = setup([p("abcdef"), list, p("end")]);
    editor.view.dispatch(placeholdersTr(editor.state, posIn(editor, "abcdef", 2), posIn(editor, "abcdef", 4), [png()]));
    expect(editor.state.doc.child(0).textContent).toBe("ab");
    expect(kinds(editor)).toEqual(["paragraph", "imageUpload", "paragraph", "bulletList", "paragraph"]);
    const at = posIn(editor, "item", 2);
    editor.view.dispatch(placeholdersTr(editor.state, at, at, [png()]));
    expect(kinds(editor)).toEqual(["paragraph", "imageUpload", "paragraph", "bulletList", "imageUpload", "paragraph"]);
  });

  it("puts an image in a column between the column's blocks", () => {
    const { editor } = setup([
      { kind: "columns", blocks: [{ kind: "column", fields: { width: 1 }, blocks: [p("leftside")] }, { kind: "column", fields: { width: 1 }, blocks: [p("right")] }] },
    ]);
    const at = posIn(editor, "leftside", 4);
    editor.view.dispatch(placeholdersTr(editor.state, at, at, [png()]));
    const col = editor.state.doc.child(0).child(0);
    const types: string[] = [];
    col.forEach((n) => types.push(n.type.name));
    expect(types).toEqual(["paragraph", "imageUpload", "paragraph"]);
  });

  it("takes the placeholder out when the upload fails, joining the paragraph it split", async () => {
    const { editor, calls, errors } = setup([p("keepme")]);
    const at = posIn(editor, "keepme", 4);
    editor.view.dispatch(placeholdersTr(editor.state, at, at, [png("bad.png")]));
    editor.view.dispatch(editor.state.tr.insertText("Z"));
    calls[0]!.done.reject(new Error("disk full"));
    await flush();
    expect(errors).toEqual(["bad.png: Error: disk full"]);
    expect(docToBlocks(editor.getJSON())).toEqual([p("keepZme")]);
  });

  it("drops a placeholder that no upload stands behind, such as one undo brings back", async () => {
    const { editor, calls } = setup([p("text")]);
    const at = posIn(editor, "text", 4);
    editor.view.dispatch(placeholdersTr(editor.state, at, at, [png()]));
    calls[0]!.done.resolve("A");
    await flush();
    expect(kinds(editor)).toContain("image");
    editor.commands.undo();
    expect(kinds(editor)).not.toContain("image");
    editor.commands.redo();
    expect(kinds(editor)).not.toContain("imageUpload");
  });

});

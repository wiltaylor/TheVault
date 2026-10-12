import { Editor } from "@tiptap/core";
import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import type { Block } from "../api";
import { FilePicker, upload } from "../blocks/MediaBlocks";
import { blocksToDoc, docToBlocks } from "./doc";
import { editorExtensions } from "./extensions";
import { PALETTE } from "./markup";

type MarkName = "bold" | "italic" | "underline" | "strike" | "code" | "highlight";

const MARK_BUTTONS: { mark: MarkName; label: string; text: string; keys: string }[] = [
  { mark: "bold", label: "Bold", text: "B", keys: "Mod+B" },
  { mark: "italic", label: "Italic", text: "I", keys: "Mod+I" },
  { mark: "underline", label: "Underline", text: "U", keys: "Mod+U" },
  { mark: "strike", label: "Strikethrough", text: "S", keys: "Mod+Shift+S" },
  { mark: "code", label: "Code", text: "</>", keys: "Mod+E" },
  { mark: "highlight", label: "Highlight", text: "H", keys: "Mod+Shift+H" },
];

const BLOCK_TYPES = [
  { value: "paragraph", label: "Paragraph" },
  { value: "h1", label: "Heading 1" },
  { value: "h2", label: "Heading 2" },
  { value: "h3", label: "Heading 3" },
  { value: "quote", label: "Quote" },
];

/** Blocks the toolbar inserts as they are, ready to fill in. */
const INSERTS: { label: string; text: string; block: () => Block }[] = [
  { label: "Insert code", text: "Code", block: () => ({ kind: "code", fields: { source: "" } }) },
  {
    label: "Insert terminal",
    text: "Terminal",
    block: () => ({ kind: "terminal", blocks: [{ kind: "command", fields: { line: "" } }] }),
  },
  {
    label: "Insert diagram",
    text: "Diagram",
    block: () => ({ kind: "mermaid", fields: { source: "graph LR\n  A --> B\n" } }),
  },
];

/**
 * The block editor for a content field (D29, D34): TipTap on ProseMirror, with a toolbar for the
 * inline marks of PRD Appendix A and the text blocks. `onChange` receives the field's blocks
 * after every edit.
 */
export function ContentEditor(props: { blocks: Block[]; label: string; onChange: (blocks: Block[]) => void }) {
  let host!: HTMLDivElement;
  let editor: Editor | undefined;
  // Bumped on every transaction so the toolbar re-reads the editor's state.
  const [tick, setTick] = createSignal(0);
  const [linkOpen, setLinkOpen] = createSignal(false);
  const [href, setHref] = createSignal("");
  const [uploadError, setUploadError] = createSignal<string | null>(null);

  onMount(() => {
    editor = new Editor({
      element: host,
      extensions: editorExtensions(),
      content: blocksToDoc(props.blocks),
      editorProps: {
        attributes: {
          class: "content-editor-doc",
          role: "textbox",
          "aria-multiline": "true",
          "aria-label": props.label,
        },
      },
      onUpdate: ({ editor }) => props.onChange(docToBlocks(editor.getJSON())),
      onTransaction: () => setTick(tick() + 1),
    });
  });
  onCleanup(() => editor?.destroy());

  const active = (name: string, attrs?: Record<string, unknown>) => {
    tick();
    return editor?.isActive(name, attrs) ?? false;
  };
  const chain = () => editor!.chain().focus();

  const blockType = () => {
    tick();
    for (const level of [1, 2, 3]) if (active("heading", { level })) return `h${level}`;
    if (active("blockquote")) return "quote";
    return "paragraph";
  };

  function setBlockType(v: string) {
    if (!editor) return;
    if (editor.isActive("blockquote") && v !== "quote") chain().lift("blockquote").run();
    if (v === "paragraph") chain().setParagraph().run();
    else if (v === "quote") chain().setParagraph().setBlockquote().run();
    else chain().setHeading({ level: Number(v.slice(1)) as 1 | 2 | 3 }).run();
  }

  const colour = () => {
    tick();
    return (editor?.getAttributes("colour").colour as string | undefined) ?? "";
  };

  function openLink() {
    setHref((editor?.getAttributes("link").href as string | undefined) ?? "");
    setLinkOpen(true);
  }

  function applyLink() {
    const url = href().trim();
    if (url) chain().extendMarkRange("link").setLink({ href: url }).run();
    else chain().extendMarkRange("link").unsetLink().run();
    // Leave the caret after the link, so typing carries on rather than replacing it.
    if (editor) editor.commands.setTextSelection(editor.state.selection.to);
    setLinkOpen(false);
  }

  function insertBlock(block: Block) {
    chain().insertContent({ type: "vaultBlock", attrs: { block } }).run();
  }

  /** Upload `file` through the assets API, then insert the block `make` builds for it. */
  async function insertUpload(file: File, make: (asset: string) => Block) {
    setUploadError(null);
    try {
      const asset = await upload(file);
      insertBlock(make(asset.id));
    } catch (e) {
      setUploadError(`Could not upload ${file.name}: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  // Toolbar buttons keep the editor's selection: pressing one must not move focus first.
  const keep = (e: MouseEvent) => e.preventDefault();

  return (
    <div class="content-editor" data-testid="content-editor">
      <div class="editor-toolbar" role="toolbar" aria-label={`${props.label} formatting`}>
        <select
          aria-label="Block type"
          value={blockType()}
          onChange={(e) => setBlockType(e.currentTarget.value)}
        >
          <For each={BLOCK_TYPES}>{(t) => <option value={t.value}>{t.label}</option>}</For>
        </select>
        <For each={MARK_BUTTONS}>
          {(b) => (
            <button
              type="button"
              class={`mark-button mark-${b.mark}`}
              aria-label={b.label}
              title={`${b.label} (${b.keys})`}
              aria-pressed={active(b.mark)}
              onMouseDown={keep}
              onClick={() => chain().toggleMark(b.mark).run()}
            >
              {b.text}
            </button>
          )}
        </For>
        <select
          aria-label="Text colour"
          value={colour()}
          onChange={(e) => {
            const c = e.currentTarget.value;
            if (c) chain().setColour(c).run();
            else chain().unsetColour().run();
          }}
        >
          <option value="">Default colour</option>
          <For each={PALETTE}>{(c) => <option value={c}>{c}</option>}</For>
        </select>
        <button type="button" aria-label="Link" aria-pressed={active("link")} onMouseDown={keep} onClick={openLink}>
          Link
        </button>
        <button
          type="button"
          aria-label="Mention a note"
          onMouseDown={keep}
          onClick={() => chain().insertMentionPicker().run()}
        >
          @
        </button>
        <button
          type="button"
          aria-label="Bullet list"
          aria-pressed={active("bulletList")}
          onMouseDown={keep}
          onClick={() => chain().toggleBulletList().run()}
        >
          •
        </button>
        <button
          type="button"
          aria-label="Numbered list"
          aria-pressed={active("orderedList")}
          onMouseDown={keep}
          onClick={() => chain().toggleOrderedList().run()}
        >
          1.
        </button>
        <button
          type="button"
          aria-label="To-do list"
          aria-pressed={active("taskList")}
          onMouseDown={keep}
          onClick={() => chain().toggleTaskList().run()}
        >
          ☐
        </button>
        <FilePicker
          label="Insert image"
          text="Image"
          accept="image/*"
          onFile={(f) => void insertUpload(f, (asset) => ({ kind: "image", fields: { asset } }))}
        />
        <FilePicker
          label="Insert file"
          text="File"
          onFile={(f) => void insertUpload(f, (asset) => ({ kind: "asset", fields: { asset } }))}
        />
        <For each={INSERTS}>
          {(b) => (
            <button type="button" aria-label={b.label} onMouseDown={keep} onClick={() => insertBlock(b.block())}>
              {b.text}
            </button>
          )}
        </For>
      </div>
      <Show when={uploadError()}>
        <p class="error" role="alert">
          {uploadError()}
        </p>
      </Show>
      <Show when={linkOpen()}>
        {/* Not a form: the editor sits inside the note's form, and forms do not nest. */}
        <div class="link-form">
          <input
            type="url"
            aria-label="Link address"
            placeholder="https://"
            value={href()}
            onInput={(e) => setHref(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                applyLink();
              } else if (e.key === "Escape") {
                e.preventDefault();
                setLinkOpen(false);
                editor?.commands.focus();
              }
            }}
            ref={(el) => queueMicrotask(() => el.focus())}
          />
          <button type="button" onClick={applyLink}>
            Apply link
          </button>
        </div>
      </Show>
      <div ref={host} class="content-editor-host" />
    </div>
  );
}

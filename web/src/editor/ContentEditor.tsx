import { Editor, type JSONContent } from "@tiptap/core";
import { Selection, TextSelection } from "@tiptap/pm/state";
import type { Node as PMNode } from "@tiptap/pm/model";
import { For, Show, createMemo, createSignal, getOwner, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import type { Block } from "../api";
import { upload } from "../blocks/MediaBlocks";
import { newCanvas } from "../canvas/canvas";
import { NEW_HEIGHT } from "../ink/ink";
import { blocksToDoc, docToBlocks } from "./doc";
import { newQueryView } from "../views/model";
import { editorExtensions } from "./extensions";
import { PALETTE } from "./markup";
import { fresh } from "./BlockHolder";
import { type DraggedBlock, matchesQuery } from "./blockTools";
import { COLUMNS_MAX, COLUMNS_MIN, newColumns } from "../blocks/columns";
import { type MenuItem, type MenuPos, SuggestMenu } from "./SlashMenu";
import type { SlotContext } from "./SlotHolder";
import { adoptOwner } from "./solidNodeView";
import { useControlBinding } from "../controls/binding";
import { CONTROL_INFO, CONTROL_KINDS, fieldKindText, fittingFields, newControl } from "../controls/model";
import { TITLE_FIELD } from "../notes";
import { ADMONITION_KINDS, PRESETS } from "../blocks/toggle";

type MarkName = "bold" | "italic" | "underline" | "strike" | "code" | "highlight";

const MARK_BUTTONS: { mark: MarkName; label: string; text: string; keys: string }[] = [
  { mark: "bold", label: "Bold", text: "B", keys: "Mod+B" },
  { mark: "italic", label: "Italic", text: "I", keys: "Mod+I" },
  { mark: "underline", label: "Underline", text: "U", keys: "Mod+U" },
  { mark: "strike", label: "Strikethrough", text: "S", keys: "Mod+Shift+S" },
  { mark: "code", label: "Code", text: "</>", keys: "Mod+E" },
  { mark: "highlight", label: "Highlight", text: "H", keys: "Mod+Shift+H" },
];


/** Blocks the toolbar and the `/` menu insert as they are, ready to fill in. */
export const INSERTS: { label: string; text: string; menu: string; keywords: string; block: () => Block }[] = [
  { label: "Insert code", text: "Code", menu: "Code", keywords: "snippet source", block: () => ({ kind: "code", fields: { source: "" } }) },
  {
    label: "Insert shell command",
    text: "Shell",
    menu: "Shell command",
    keywords: "bash sh terminal console command",
    block: () => ({ kind: "code", fields: { language: "bash", source: "" } }),
  },
  {
    label: "Insert output",
    text: "Output",
    menu: "Output",
    keywords: "stdout terminal console result",
    block: () => ({ kind: "code", fields: { language: "stdout", source: "" } }),
  },
  {
    label: "Insert diagram",
    text: "Diagram",
    menu: "Mermaid diagram",
    keywords: "mermaid chart flow",
    block: () => ({ kind: "mermaid", fields: { source: "graph LR\n  A --> B\n" } }),
  },
  {
    label: "Insert drawing",
    text: "Ink",
    menu: "Ink drawing",
    keywords: "ink sketch draw pen",
    block: () => ({ kind: "ink", fields: { height: NEW_HEIGHT }, blocks: [] }),
  },
  { label: "Insert query view", text: "Query", menu: "Query view", keywords: "query table list cards kanban", block: newQueryView },
  { label: "Insert canvas", text: "Canvas", menu: "Canvas", keywords: "board whiteboard", block: newCanvas },
];

/**
 * `doc` ending in a paragraph, so there is always a line to type on after the last block. An
 * empty paragraph at the end is not content: it is dropped when the document is read back.
 */
function withTrailingParagraph(doc: JSONContent): JSONContent {
  const content = doc.content ?? [];
  return content.at(-1)?.type === "paragraph" ? doc : { ...doc, content: [...content, { type: "paragraph" }] };
}

const uploadFailed = (file: File, e: unknown) => `Could not upload ${file.name}: ${e instanceof Error ? e.message : String(e)}`;

/** A template being edited: its property blocks and the fields `{{` offers. */
export interface TemplateEditing {
  slots: SlotContext;
  /** Fields the template does not place yet, `title` included: the `/` menu's Property section. */
  unplaced: () => { name: string; label: string }[];
  /** Every field a `{{name}}` reference can name, `title` first. */
  fields: () => { name: string; label: string }[];
}

/** What the editor hands its owner once it exists. */
export interface EditorHandle {
  editor: Editor;
}

interface MenuState {
  from: number;
  to: number;
  query: string;
}

/**
 * The block editor for a content field (D29, D34) and for a template's tabs: TipTap on
 * ProseMirror. Typing `/` opens a menu of blocks to insert, column layouts among them; every
 * block, at the top level or in a column, has a drag handle and a "+" that opens the same menu,
 * and Alt+ArrowUp / Alt+ArrowDown move the current block within its column or the top level. Selecting text shows a bubble with the inline marks of PRD Appendix A, a link and a
 * mention. `onChange` receives the field's blocks
 * after every edit; with `doc`, the editor edits that document and `onDoc` receives it instead.
 */
export function ContentEditor(props: {
  blocks: Block[];
  label: string;
  onChange: (blocks: Block[]) => void;
  doc?: JSONContent;
  onDoc?: (doc: JSONContent) => void;
  template?: TemplateEditing;
  onReady?: (handle: EditorHandle) => void;
  onDrag?: (block: DraggedBlock | null) => void;
  /** Ids of `/` menu items to leave out, such as "canvas" in a canvas card. */
  exclude?: string[];
  /** Escape pressed in the editor when no menu, picker or field inside it took it. */
  onEscape?: () => void;
  /**
   * The note and field this editor edits, as `id/field`: this device remembers its toggles open
   * or closed under it (blocks/toggle.ts). Without it, they start as new ones do on every load.
   */
  toggleScope?: string;
}) {
  // The fields a control inserted here can bind: the note's, or the template's type's.
  const binding = useControlBinding();
  let host!: HTMLDivElement;
  let wrap!: HTMLDivElement;
  let fileInput!: HTMLInputElement;
  let editor: Editor | undefined;
  // Bumped on every transaction so the toolbar re-reads the editor's state.
  const [tick, setTick] = createSignal(0);
  const [linkOpen, setLinkOpen] = createSignal(false);
  const [href, setHref] = createSignal("");
  const [uploadError, setUploadError] = createSignal<string | null>(null);
  // The open menu: which trigger, where its text is, and the highlighted item.
  const [slash, setSlash] = createSignal<MenuState | null>(null);
  const [fieldMenu, setFieldMenu] = createSignal<MenuState | null>(null);
  const [activeItem, setActiveItem] = createSignal(0);
  // A follow-up list the `/` menu shows in place of its items, such as the column counts.
  const [subMenu, setSubMenu] = createSignal<MenuItem[] | null>(null);
  // A menu closed with Escape stays closed until its trigger is typed again.
  const dismissed: Record<string, number | null> = { slash: null, field: null };
  // The file a menu item asked for goes into a block this builds.
  let onPicked: ((asset: string) => Block) | null = null;
  // Where "+" typed a `/` for the menu; Escape takes it out again.
  let plusSlash: number | null = null;
  // The last Escape a menu took, which must not also reach `onEscape`.
  let menuEscape: KeyboardEvent | null = null;
  // Focus is inside the editor or its bubble, which shows only then.
  const [focused, setFocused] = createSignal(false);
  // Bumped on scroll and resize, so the floating menu and bubble follow their text.
  const [layout, setLayout] = createSignal(0);
  const relayout = () => setLayout(layout() + 1);
  window.addEventListener("scroll", relayout, true);
  window.addEventListener("resize", relayout);
  onCleanup(() => {
    window.removeEventListener("scroll", relayout, true);
    window.removeEventListener("resize", relayout);
  });

  function track(kind: "slash" | "field", set: (s: MenuState | null) => void) {
    return (st: MenuState | null) => {
      if (!st || dismissed[kind] !== st.from) dismissed[kind] = null;
      const next = st && dismissed[kind] === null ? st : null;
      set(next);
      if (!next) {
        setActiveItem(0);
        if (kind === "slash") setSubMenu(null);
      }
    };
  }

  const owner = getOwner();
  onMount(() => {
    const keys = (kind: "slash" | "field") => (e: KeyboardEvent) => onMenuKey(kind, e);
    editor = new Editor({
      element: host,
      extensions: editorExtensions({
        slash: { onChange: track("slash", setSlash), onKey: keys("slash") },
        template: props.template && {
          slots: props.template.slots,
          fieldMenu: { onChange: track("field", setFieldMenu), onKey: keys("field") },
        },
        handle: { onDrag: (b) => props.onDrag?.(b), onAdd: addAfter },
        // A pasted or dropped image is uploaded and goes in where the caret or the drop was.
        images: {
          upload: async (file) => (await upload(file)).id,
          onStart: () => setUploadError(null),
          onError: (file, e) => setUploadError(uploadFailed(file, e)),
        },
        toggleScope: () => props.toggleScope,
      }),
      content: withTrailingParagraph(props.doc ?? blocksToDoc(props.blocks)),
      editorProps: {
        attributes: {
          class: "content-editor-doc",
          role: "textbox",
          "aria-multiline": "true",
          "aria-label": props.label,
        },
      },
      onUpdate: ({ editor }) => {
        if (props.onDoc) props.onDoc(editor.getJSON());
        else props.onChange(docToBlocks(editor.getJSON()));
      },
      onTransaction: () => setTick(tick() + 1),
    });
    adoptOwner(editor, owner);
    props.onReady?.({ editor });
  });
  onCleanup(() => editor?.destroy());

  // ------------------------------------------------------------------------------------------
  // The menus

  const openMenu = (): { kind: "slash" | "field"; state: MenuState } | null => {
    const f = fieldMenu();
    if (f) return { kind: "field", state: f };
    const s = slash();
    return s ? { kind: "slash", state: s } : null;
  };

  function slashItems(): MenuItem[] {
    const range = () => {
      const s = slash();
      return s ? { from: s.from, to: s.to } : null;
    };
    const run = (fn: (c: ReturnType<typeof chain>) => ReturnType<typeof chain>) => () => {
      const r = range();
      if (r) fn(chain().deleteRange(r)).run();
    };
    const textItems: MenuItem[] = [
      { id: "paragraph", label: "Text", section: "Text", keywords: "paragraph plain", run: run((c) => c.setParagraph()) },
      ...([1, 2, 3] as const).map((level) => ({
        id: `h${level}`,
        label: `Heading ${level}`,
        section: "Text",
        keywords: `h${level} title`,
        hint: "#".repeat(level),
        run: run((c) => c.setParagraph().setHeading({ level })),
      })),
      { id: "bullet", label: "Bullet list", section: "Text", keywords: "unordered ul", run: run((c) => c.toggleBulletList()) },
      { id: "numbered", label: "Numbered list", section: "Text", keywords: "ordered ol", run: run((c) => c.toggleOrderedList()) },
      { id: "todo", label: "To-do list", section: "Text", keywords: "task checkbox check", run: run((c) => c.toggleTaskList()) },
      { id: "quote", label: "Quote", section: "Text", keywords: "blockquote", run: run((c) => c.setParagraph().setBlockquote()) },
      { id: "mention", label: "Mention a note", section: "Text", keywords: "reference note @", run: run((c) => c.insertMentionPicker()) },
      {
        id: "toggle",
        label: "Toggle",
        section: "Text",
        keywords: "collapse expand fold details disclosure",
        hint: ">>",
        run: () => place(toggleJSON(0)),
      },
      ...([1, 2, 3] as const).map((level) => ({
        id: `toggle-h${level}`,
        label: `Toggle heading ${level}`,
        section: "Text",
        keywords: "collapse expand fold",
        run: () => place(toggleJSON(level)),
      })),
      {
        id: "callout",
        label: "Callout",
        section: "Text",
        keywords: "admonition note info tip success warning danger question quote alert",
        run: () => {},
        submenu: () =>
          ADMONITION_KINDS.map((kind) => ({
            id: `callout-${kind}`,
            label: PRESETS[kind].label,
            section: "Callout: choose a kind",
            keywords: `${kind} callout admonition`,
            hint: PRESETS[kind].colour,
            run: () => place(admonitionJSON(kind)),
          })),
      },
    ];
    const blockItems: MenuItem[] = [
      ...INSERTS.map((b) => ({
        id: b.text.toLowerCase(),
        label: b.menu,
        section: "Blocks",
        keywords: b.keywords,
        run: () => place({ type: "vaultBlock", attrs: { block: fresh(b.block()) } }),
      })),
      {
        id: "image",
        label: "Image",
        section: "Blocks",
        keywords: "picture photo upload",
        run: run((c) => {
          pickFile("image/*", (asset) => ({ kind: "image", fields: { asset } }));
          return c;
        }),
        // In a template, an image can also show an asset field of each note: a follow-up choice.
        submenu: props.template ? imageItems : undefined,
      },
      {
        id: "file",
        label: "File",
        section: "Blocks",
        keywords: "asset attachment upload",
        run: run((c) => {
          pickFile("", (asset) => ({ kind: "asset", fields: { asset } }));
          return c;
        }),
      },
    ];
    // Layouts go at the top level only: a column holds no columns.
    const s = slash();
    const inColumn = !!editor && !!s && editor.state.doc.resolve(s.from).depth > 1;
    const columnItem = (n: number): MenuItem => ({
      id: `columns-${n}`,
      label: `${n} columns`,
      section: "Layout",
      keywords: "columns layout side by side split",
      run: () => place(columnsJSON(n)),
    });
    const layoutItems: MenuItem[] = inColumn
      ? []
      : [
          ...[2, 3, 4].map(columnItem),
          {
            id: "columns-pick",
            label: "Columns…",
            section: "Layout",
            keywords: "columns layout side by side split custom count five six 5 6",
            hint: `up to ${COLUMNS_MAX}`,
            run: () => {},
            submenu: () => Array.from({ length: COLUMNS_MAX - COLUMNS_MIN + 1 }, (_, i) => columnItem(i + COLUMNS_MIN)),
          },
        ];
    const properties: MenuItem[] = (props.template?.unplaced() ?? []).map((f) => ({
      id: `property-${f.name}`,
      label: f.label,
      section: "Property",
      keywords: `${f.name} field property slot`,
      hint: f.name,
      run: () => place({ type: "templateSlot", attrs: { slot: f.name, label: null } }),
    }));
    const all = [...textItems, ...blockItems, ...layoutItems, ...controlItems(), ...properties];
    const exclude = props.exclude ?? [];
    return exclude.length ? all.filter((i) => !exclude.includes(i.id)) : all;
  }

  /**
   * A template's image: a file uploaded now, shown on every note, or an asset field of the type,
   * which shows each note's own image (docs/vault-format.md, "Images from fields").
   */
  function imageItems(): MenuItem[] {
    const section = "Image";
    const uploadItem: MenuItem = {
      id: "image-upload",
      label: "Upload an image",
      section,
      keywords: "fixed file",
      hint: "same on every note",
      run: () => {
        const s = slash();
        if (s) chain().deleteRange({ from: s.from, to: s.to }).run();
        pickFile("image/*", (asset) => ({ kind: "image", fields: { asset } }));
      },
    };
    const fields = (binding?.type()?.fields ?? []).filter((f) => f.kind === "asset");
    const fieldItems: MenuItem[] = fields.length
      ? fields.map((f) => ({
          id: `image-field-${f.name}`,
          label: `From field: ${f.label}`,
          section,
          keywords: `${f.name} field property asset`,
          hint: f.multiple ? "each note's images" : "each note's image",
          run: () => place({ type: "vaultBlock", attrs: { block: fresh({ kind: "image", fields: { field: f.name } }) } }),
        }))
      : [{ id: "image-field-none", label: "From field: the type has no asset field", section, run: () => {} }];
    return [uploadItem, ...fieldItems];
  }

  /**
   * The Controls section: a form control of each kind. Choosing one asks which field it binds,
   * from the fields of the note's type (or the template's) that it fits.
   */
  function controlItems(): MenuItem[] {
    const type = binding?.type();
    if (!type) return [];
    const fields = [TITLE_FIELD, ...type.fields];
    return CONTROL_KINDS.map((kind) => {
      const info = CONTROL_INFO[kind];
      return {
        id: `control-${kind}`,
        label: info.label,
        section: "Controls",
        keywords: `control form ${kind} ${info.keywords}`,
        run: () => {},
        submenu: () => {
          const fitting = fittingFields(kind, fields);
          const section = `${info.label}: choose a field`;
          if (!fitting.length) return [{ id: `control-${kind}-none`, label: `No field fits a ${info.label.toLowerCase()}`, section, run: () => {} }];
          return fitting.map((f) => ({
            id: `control-${kind}-${f.name}`,
            label: f.label,
            section,
            keywords: f.name,
            hint: fieldKindText(f),
            run: () => place({ type: "vaultBlock", attrs: { block: fresh(newControl(kind, f.name)) } }),
          }));
        },
      };
    });
  }

  /**
   * Put block `json` where the `/` menu was typed: in place of its paragraph when that is now
   * empty, else after it. The caret goes to a paragraph after the block, ready to type on.
   */
  function place(json: JSONContent) {
    const s = slash();
    if (!editor || !s) return;
    const { schema, view } = editor;
    let tr = view.state.tr.delete(s.from, s.to);
    // The paragraph the menu was typed in: at the top level, or in a column.
    const $pos = tr.doc.resolve(s.from);
    const start = $pos.before();
    const para = $pos.parent;
    const node = schema.nodeFromJSON(json);
    const from = para.content.size === 0 ? start : start + para.nodeSize;
    const to = para.content.size === 0 ? start + para.nodeSize : from;
    const next = tr.doc.resolve(to).nodeAfter;
    // At the end, add an empty line to type on after the block. Elsewhere the caret goes to the
    // start of the block after it, and no empty paragraph is left in the middle of the content.
    tr = tr.replaceWith(from, to, next ? [node] : [node, schema.nodes.paragraph!.create()]);
    // A new layout takes the caret into its first column, a toggle into its summary and a
    // callout into its first line.
    if (node.type.name === "columns") tr.setSelection(TextSelection.create(tr.doc, from + 3));
    else if (node.type.name === "toggle") tr.setSelection(TextSelection.create(tr.doc, from + 2));
    else if (node.type.name === "admonition") tr.setSelection(TextSelection.create(tr.doc, from + 1 + node.child(0).nodeSize + 1));
    else tr.setSelection(Selection.near(tr.doc.resolve(from + node.nodeSize)));
    view.dispatch(tr.scrollIntoView());
    view.focus();
  }

  /** The editor node for a new toggle, its summary a heading of `level` or plain text for 0. */
  function toggleJSON(level: number): JSONContent {
    return { type: "toggle", content: [{ type: "toggleSummary", attrs: { level } }] };
  }

  /** The editor node for a new callout of `kind`, with a line to type on. */
  function admonitionJSON(kind: string): JSONContent {
    return { type: "admonition", attrs: { kind }, content: [{ type: "admonitionTitle" }, { type: "paragraph" }] };
  }

  /** The editor node for a new layout of `n` equal columns, each with a line to type on. */
  function columnsJSON(n: number): JSONContent {
    return {
      type: "columns",
      content: (newColumns(n).blocks ?? []).map(() => ({ type: "column", attrs: { width: 1 }, content: [{ type: "paragraph" }] })),
    };
  }

  function fieldItems(): MenuItem[] {
    return (props.template?.fields() ?? []).map((f) => ({
      id: `ref-${f.name}`,
      label: f.label,
      section: "Show a field",
      keywords: f.name,
      hint: `{{${f.name}}}`,
      run: () => {
        const s = fieldMenu();
        if (s) chain().deleteRange({ from: s.from, to: s.to }).insertContent({ type: "fieldRef", attrs: { name: f.name } }).run();
      },
    }));
  }

  const menuItems = createMemo(() => {
    const m = openMenu();
    if (!m) return [];
    const sub = m.kind === "slash" ? subMenu() : null;
    if (sub) return sub;
    const all = m.kind === "slash" ? slashItems() : fieldItems();
    return all.filter((i) => matchesQuery(`${i.label} ${i.section} ${i.keywords ?? ""}`, m.state.query));
  });

  /**
   * Where the menu goes, in the viewport: under the trigger, or over it when there is more room
   * there, never past an edge, and never taller than the room it has.
   */
  const menuPos = (): MenuPos => {
    tick();
    layout();
    const m = openMenu();
    if (!m || !editor) return { top: 0, left: 0, maxHeight: 320 };
    const at = editor.view.coordsAtPos(m.state.from);
    const margin = 8;
    const width = Math.min(352, window.innerWidth - 2 * margin);
    const left = Math.max(margin, Math.min(at.left, window.innerWidth - width - margin));
    const below = window.innerHeight - at.bottom - margin - 4;
    const above = at.top - margin - 4;
    if (below >= 240 || below >= above) return { top: at.bottom + 4, left, maxHeight: Math.min(320, below) };
    return { bottom: window.innerHeight - at.top + 4, left, maxHeight: Math.min(320, above) };
  };

  function choose(item: MenuItem) {
    if (item.submenu) {
      // A follow-up choice: the menu lists it, and Escape goes back.
      setSubMenu(item.submenu());
      setActiveItem(0);
      return;
    }
    setSubMenu(null);
    plusSlash = null;
    item.run();
    setSlash(null);
    setFieldMenu(null);
    setActiveItem(0);
  }

  function onMenuKey(kind: "slash" | "field", e: KeyboardEvent): boolean {
    const m = openMenu();
    if (!m || m.kind !== kind) return false;
    const items = menuItems();
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      const n = items.length;
      if (n) setActiveItem((activeItem() + (e.key === "ArrowDown" ? 1 : n - 1)) % n);
      return true;
    }
    if (e.key === "Enter" || e.key === "Tab") {
      const item = items[Math.min(activeItem(), items.length - 1)];
      if (!item) return false;
      choose(item);
      return true;
    }
    if (e.key === "Escape") {
      menuEscape = e;
      if (kind === "slash" && subMenu()) {
        setSubMenu(null);
        setActiveItem(0);
        return true;
      }
      if (kind === "slash" && plusSlash === m.state.from && editor) {
        // The `/` came from "+", not from typing: take it out again.
        plusSlash = null;
        editor.view.dispatch(editor.state.tr.delete(m.state.from, m.state.to));
        setSlash(null);
        return true;
      }
      dismissed[kind] = m.state.from;
      if (kind === "slash") setSlash(null);
      else setFieldMenu(null);
      return true;
    }
    return false;
  }

  function pickFile(accept: string, make: (asset: string) => Block) {
    onPicked = make;
    fileInput.accept = accept;
    fileInput.click();
  }

  /**
   * "+" beside block `node` at `pos`: open the block menu on an empty line after it (the block
   * itself when it is an empty paragraph), by typing the `/` that opens it.
   */
  function addAfter(pos: number, node: PMNode) {
    if (!editor) return;
    const { state, view, schema } = editor;
    let tr = state.tr;
    let at: number;
    if (node.type.name === "paragraph" && node.content.size === 0) {
      at = pos + 1;
    } else {
      const end = pos + node.nodeSize;
      const next = state.doc.resolve(end).nodeAfter;
      if (next?.type.name === "paragraph" && next.content.size === 0) at = end + 1;
      else {
        tr = tr.insert(end, schema.nodes.paragraph!.create());
        at = end + 1;
      }
    }
    tr = tr.insertText("/", at);
    tr.setSelection(TextSelection.create(tr.doc, at + 1));
    dismissed.slash = null;
    plusSlash = at;
    view.dispatch(tr.scrollIntoView());
    view.focus();
    setSlash({ from: at, to: at + 1, query: "" });
  }

  /** A non-empty text selection, which the bubble formats. */
  const textSelected = () => {
    tick();
    const sel = editor?.state.selection;
    return !!sel && !sel.empty && sel instanceof TextSelection;
  };

  const bubbleOpen = () => focused() && (textSelected() || linkOpen()) && !openMenu();

  /** The bubble sits just above the selection, or just below it when there is no room above. */
  const bubblePos = (): { top?: number; bottom?: number; left: number } => {
    tick();
    layout();
    if (!editor) return { top: 0, left: 0 };
    const { from, to } = editor.state.selection;
    const start = editor.view.coordsAtPos(from);
    const end = editor.view.coordsAtPos(to);
    const margin = 8;
    const width = Math.min(bubble?.offsetWidth || 420, window.innerWidth - 2 * margin);
    const left = Math.max(margin, Math.min(start.left, window.innerWidth - width - margin));
    // The app's top bar covers the top of the window.
    const topbar = document.querySelector(".topbar")?.getBoundingClientRect().bottom ?? 0;
    // Its own height: on a phone it wraps to two rows.
    const height = (bubble?.offsetHeight || 44) + 8;
    if (start.top - height > Math.max(margin, topbar)) return { bottom: window.innerHeight - start.top + 6, left };
    return { top: end.bottom + 6, left };
  };

  const active = (name: string, attrs?: Record<string, unknown>) => {
    tick();
    return editor?.isActive(name, attrs) ?? false;
  };
  // TipTap's focus() hands focus back a frame later, and keys typed before then are lost. Taking it
  // now also makes the focus() in the chain a no-op.
  const chain = () => {
    readSelection();
    editor!.view.focus();
    return editor!.chain().focus();
  };

  /**
   * Take in the page's current selection before acting on it. ProseMirror reads a selection
   * change a moment after the browser makes it, so a selection extended by keyboard and acted on
   * at once (Shift+Arrow, then Bold) would otherwise format the selection as it was a key earlier.
   */
  const readSelection = () =>
    (editor?.view as unknown as { domObserver?: { flush?: () => void } } | undefined)?.domObserver?.flush?.();

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
    chain().insertContent({ type: "vaultBlock", attrs: { block: fresh(block) } }).run();
  }

  /** Upload `file` through the assets API, then insert the block `make` builds for it. */
  async function insertUpload(file: File, make: (asset: string) => Block) {
    setUploadError(null);
    try {
      const asset = await upload(file);
      insertBlock(make(asset.id));
    } catch (e) {
      setUploadError(uploadFailed(file, e));
    }
  }

  let bubble: HTMLDivElement | undefined;
  /** Focus leaving for somewhere outside both the editor and its bubble. */
  function onFocusOut(e: FocusEvent) {
    const to = e.relatedTarget;
    if (to instanceof Node && (wrap.contains(to) || bubble?.contains(to))) return;
    setFocused(false);
    setLinkOpen(false);
  }

  // Bubble buttons keep the editor's selection: pressing one must not move focus first.
  const keep = (e: MouseEvent) => {
    e.preventDefault();
    readSelection();
  };

  return (
    <div
      class="content-editor"
      data-testid="content-editor"
      ref={wrap}
      onFocusIn={() => setFocused(true)}
      onFocusOut={onFocusOut}
      onKeyDown={(e) => {
        if (e.key !== "Escape" || !props.onEscape || e === menuEscape) return;
        // A field inside a block, or the mention picker, keeps its own Escape.
        if (e.target instanceof Element && e.target.closest("input, textarea, select, .mention-picker")) return;
        props.onEscape();
      }}
    >
      <Show when={uploadError()}>
        <p class="error" role="alert">
          {uploadError()}
        </p>
      </Show>
      <Show when={bubbleOpen()}>
        <Portal>
          <div
            class="editor-bubble"
            role="toolbar"
            aria-label={`${props.label} formatting`}
            style={{
              top: bubblePos().top === undefined ? undefined : `${bubblePos().top}px`,
              bottom: bubblePos().bottom === undefined ? undefined : `${bubblePos().bottom}px`,
              left: `${bubblePos().left}px`,
            }}
            ref={bubble}
            onFocusOut={onFocusOut}
          >
            <Show
              when={linkOpen()}
              fallback={
                <>
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
                    title="Text colour"
                    value={colour()}
                    onChange={(e) => {
                      const c = e.currentTarget.value;
                      if (c) chain().setColour(c).run();
                      else chain().unsetColour().run();
                    }}
                  >
                    <option value="">Colour</option>
                    <For each={PALETTE}>{(c) => <option value={c}>{c}</option>}</For>
                  </select>
                  <button
                    type="button"
                    aria-label="Link"
                    title="Link"
                    aria-pressed={active("link")}
                    onMouseDown={keep}
                    onClick={openLink}
                  >
                    Link
                  </button>
                  <button
                    type="button"
                    aria-label="Mention a note"
                    title="Mention a note"
                    onMouseDown={keep}
                    onClick={() => chain().insertMentionPicker().run()}
                  >
                    @
                  </button>
                </>
              }
            >
              {/* Not a form: the editor sits inside the note's page, and forms do not nest. */}
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
                    editor?.view.focus();
                  }
                }}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <button type="button" onClick={applyLink}>
                Apply link
              </button>
            </Show>
          </div>
        </Portal>
      </Show>
      <div ref={host} class="content-editor-host" />
      <input
        ref={fileInput}
        type="file"
        hidden
        onChange={(e) => {
          const file = e.currentTarget.files?.[0];
          e.currentTarget.value = "";
          const make = onPicked;
          onPicked = null;
          if (file && make) void insertUpload(file, make);
        }}
      />
      <Show when={openMenu()}>
        {(m) => (
          <Portal>
          <SuggestMenu
            label={m().kind === "slash" ? "Insert block" : "Insert field"}
            items={menuItems()}
            active={Math.min(activeItem(), Math.max(0, menuItems().length - 1))}
            pos={menuPos()}
            onHover={setActiveItem}
            onChoose={choose}
          />
          </Portal>
        )}
      </Show>
    </div>
  );
}

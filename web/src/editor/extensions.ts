// The editor's schema: TipTap's text blocks and marks, plus TheVault's colour mark, mentions,
// the mention picker, column layouts and a holder for content blocks this editor does not edit
// yet (D29, D34).

import { type AnyExtension, Extension, InputRule, Mark, Node, mergeAttributes, nodeInputRule } from "@tiptap/core";
import Code from "@tiptap/extension-code";
import Highlight from "@tiptap/extension-highlight";
import { TaskItem, TaskList } from "@tiptap/extension-list";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";
import StarterKit from "@tiptap/starter-kit";
import { followLink, wantsNewTab } from "../workspace/events";
import { BlockHolder } from "./BlockHolder";
import {
  type BlockHandleOptions,
  EdgeDropColumns,
  MoveBlock,
  SteadyFocus,
  type SuggestOptions,
  blockHandle,
  matchFieldRef,
  placeholder,
  matchSlash,
  suggestion,
} from "./blockTools";
import { Column, ColumnRules, Columns, VaultDoc } from "./columns";
import { FieldRefChip } from "./FieldRefChip";
import { type SlotContext, SlotHolder } from "./SlotHolder";
import { MentionChip } from "./MentionChip";
import { MentionPicker } from "./MentionPicker";
import { PALETTE, placeholderMarkup } from "./markup";
import { PlaceholderChip, placeholderActions } from "./PlaceholderChip";
import { ImageUpload, type ImageUploadOptions } from "./pasteImages";
import { AT_TRIGGER } from "./trigger";
import { solidNodeView } from "./solidNodeView";

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    colour: {
      setColour: (colour: string) => ReturnType;
      unsetColour: () => ReturnType;
    };
    mentionPicker: {
      /** Open the mention picker at the caret; `trigger` is what typing it took, `[[` or `@`. */
      insertMentionPicker: (trigger?: "[[" | "@") => ReturnType;
    };
  }
}

/** `{red}text{/}`: text in a palette colour (D41, OQ-S4). */
export const Colour = Mark.create({
  name: "colour",
  addAttributes() {
    return {
      colour: {
        default: null,
        parseHTML: (el) => el.getAttribute("data-colour"),
        renderHTML: (attrs) => ({ "data-colour": attrs.colour, class: `colour-${attrs.colour}` }),
      },
    };
  },
  parseHTML() {
    return [{ tag: "span[data-colour]", getAttrs: (el) => (PALETTE.includes(el.getAttribute("data-colour") as never) ? null : false) }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["span", HTMLAttributes, 0];
  },
  addCommands() {
    return {
      setColour:
        (colour) =>
        ({ commands }) =>
          commands.setMark(this.name, { colour }),
      unsetColour:
        () =>
        ({ commands }) =>
          commands.unsetMark(this.name),
    };
  },
});

/** `[[ULID]]` or `[[ULID|label]]`: a mention of a note, shown with its live title. */
export const Mention = Node.create({
  name: "mention",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,
  addAttributes() {
    return {
      id: { default: null, parseHTML: (el) => el.getAttribute("data-id") },
      label: { default: null, parseHTML: (el) => el.getAttribute("data-label") },
    };
  },
  parseHTML() {
    return [{ tag: "span[data-mention]" }];
  },
  renderHTML({ node, HTMLAttributes }) {
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        "data-mention": "",
        "data-id": node.attrs.id,
        "data-label": node.attrs.label,
      }),
      `[[${node.attrs.id}]]`,
    ];
  },
  renderText({ node }) {
    return node.attrs.label ?? `[[${node.attrs.id}]]`;
  },
  addNodeView() {
    return solidNodeView(MentionChip);
  },
});

/**
 * `[[+type|title]]`: a placeholder link to a note of `type` titled `title` that does not exist
 * yet. Clicking it creates the note and turns it into a mention (see `FollowLinks`).
 */
export const PlaceholderLink = Node.create({
  name: "placeholderLink",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,
  addAttributes() {
    return {
      noteType: { default: "", parseHTML: (el) => el.getAttribute("data-type") },
      title: { default: "", parseHTML: (el) => el.getAttribute("data-title") },
    };
  },
  parseHTML() {
    return [{ tag: "span[data-placeholder-link][data-title]" }];
  },
  renderHTML({ node }) {
    return [
      "span",
      { "data-placeholder-link": "", "data-type": node.attrs.noteType, "data-title": node.attrs.title },
      placeholderMarkup(node.attrs.noteType, node.attrs.title),
    ];
  },
  renderText({ node }) {
    return node.attrs.title;
  },
  addNodeView() {
    return solidNodeView(PlaceholderChip);
  },
});

/** The placeholder link a mouse event in the editor is on, with what following it does. */
function placeholderAt(view: EditorView, e: MouseEvent): ((background: boolean) => void) | null {
  const el = e.target instanceof Element ? e.target.closest("[data-placeholder-link]") : null;
  return el && view.dom.contains(el) ? (placeholderActions.get(el) ?? null) : null;
}

/** The link or mention a mouse event in the editor is on, as its `<a>`. */
function linkAt(view: EditorView, e: MouseEvent): HTMLAnchorElement | null {
  const a = e.target instanceof Element ? e.target.closest("a[href]") : null;
  return a instanceof HTMLAnchorElement && view.dom.contains(a) ? a : null;
}

/**
 * Links and mentions in the editor open when clicked, though the browser follows no link in
 * editable text: a click opens a note in the pane, Ctrl/Cmd+click or a middle-click opens it in
 * a background tab, and a link to another site opens in a new browser tab. A click that selects
 * (a drag across the link, a double click) only selects; the caret goes into a link's text
 * from beside it or with the keyboard, and the selection bubble edits the link.
 */
export const FollowLinks = Extension.create({
  name: "followLinks",
  addProseMirrorPlugins() {
    const follow = (view: EditorView, e: MouseEvent, background: boolean) => {
      const placeholder = view.editable ? placeholderAt(view, e) : null;
      if (placeholder) {
        e.preventDefault();
        placeholder(background);
        return true;
      }
      const a = linkAt(view, e);
      if (!a || !followLink(a.getAttribute("href")!, a, background)) return false;
      e.preventDefault();
      return true;
    };
    // Where the last press went down: a click that ends elsewhere was a drag that selected.
    let down: { x: number; y: number } | null = null;
    return [
      new Plugin({
        key: new PluginKey("followLinks"),
        props: {
          handleDOMEvents: {
            mousedown: (view, e) => {
              down = { x: e.clientX, y: e.clientY };
              if (!linkAt(view, e) && !placeholderAt(view, e)) return false;
              // A middle press on a link would paste or start the browser's autoscroll, and a
              // Ctrl/Cmd press would select the block around it.
              if (e.button !== 1 && !(e.button === 0 && wantsNewTab(e))) return false;
              e.preventDefault();
              return true;
            },
            click: (view, e) => {
              if (e.defaultPrevented || e.button !== 0 || e.altKey || e.shiftKey || e.detail > 1) return false;
              if (down && Math.hypot(e.clientX - down.x, e.clientY - down.y) > 3) return false;
              return follow(view, e, wantsNewTab(e));
            },
            auxclick: (view, e) => e.button === 1 && !e.defaultPrevented && follow(view, e, true),
          },
        },
      }),
    ];
  },
});

/**
 * A search box standing where a mention will go. Typing `[[`, typing `@` at the start of a word,
 * or pressing the toolbar's mention button inserts one; choosing a note replaces it with a
 * mention, and choosing "New <type>" with a placeholder link. It is never saved.
 */
export const MentionPickerNode = Node.create({
  name: "mentionPicker",
  group: "inline",
  inline: true,
  atom: true,
  selectable: false,
  addAttributes() {
    // What opened it: abandoning a picker typed as `@` leaves the `@` as text.
    return { trigger: { default: "[[", rendered: false } };
  },
  parseHTML() {
    return [];
  },
  renderHTML() {
    return ["span", { "data-mention-picker": "" }];
  },
  addCommands() {
    return {
      insertMentionPicker:
        (trigger = "[[") =>
        ({ commands }) =>
          commands.insertContent({ type: this.name, attrs: { trigger } }),
    };
  },
  addInputRules() {
    const type = this.type;
    return [
      nodeInputRule({ find: /\[\[$/, type, getAttributes: { trigger: "[[" } }),
      // `@` at the start of a word: the `@` itself becomes the picker.
      new InputRule({
        find: AT_TRIGGER,
        handler: ({ state, range }) => {
          const at = state.doc.textBetween(range.to - 1, range.to) === "@" ? range.to - 1 : range.to;
          state.tr.replaceWith(at, range.to, type.create({ trigger: "@" })).scrollIntoView();
        },
      }),
    ];
  },
  addNodeView() {
    return solidNodeView(MentionPicker);
  },
});

/** A content block this editor keeps but does not edit: an image, code, a diagram and so on. */
export const VaultBlock = Node.create({
  name: "vaultBlock",
  group: "block",
  atom: true,
  selectable: true,
  draggable: true,
  addAttributes() {
    return { block: { default: null, rendered: false } };
  },
  parseHTML() {
    return [];
  },
  renderHTML({ node }) {
    return ["div", { "data-vault-block": node.attrs.block?.kind ?? "" }];
  },
  addNodeView() {
    return solidNodeView(BlockHolder);
  },
});

/**
 * `slot "<field>"` in a template tab: a property block, which the note page renders as the
 * field's label and its editor. Only the template editor has it.
 */
export function templateSlot(ctx: SlotContext) {
  return Node.create({
    name: "templateSlot",
    group: "block",
    atom: true,
    selectable: true,
    draggable: true,
    addAttributes() {
      return {
        slot: { default: "title", parseHTML: (el) => el.getAttribute("data-template-slot") },
        label: { default: null, parseHTML: (el) => el.getAttribute("data-label") },
      };
    },
    parseHTML() {
      return [{ tag: "div[data-template-slot]" }];
    },
    renderHTML({ node }) {
      const attrs: Record<string, string> = { "data-template-slot": node.attrs.slot };
      if (node.attrs.label) attrs["data-label"] = node.attrs.label;
      return ["div", attrs];
    },
    addNodeView() {
      return solidNodeView((props) => SlotHolder({ ...props, ctx }));
    },
  });
}

/** `{{name}}` in a template's text: the shown note's value of a field. Only templates have it. */
export const FieldRef = Node.create({
  name: "fieldRef",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,
  addAttributes() {
    return { name: { default: "title", parseHTML: (el) => el.getAttribute("data-field-ref") } };
  },
  parseHTML() {
    return [{ tag: "span[data-field-ref]" }];
  },
  renderHTML({ node }) {
    return ["span", { "data-field-ref": node.attrs.name }, `{{${node.attrs.name}}}`];
  },
  renderText({ node }) {
    return `{{${node.attrs.name}}}`;
  },
  addInputRules() {
    // `{{name}}` typed out in full becomes a reference, as choosing it from the menu does.
    return [
      nodeInputRule({
        find: /\{\{([A-Za-z_][A-Za-z0-9_]*)\}\}$/,
        type: this.type,
        getAttributes: (m) => ({ name: m[1] }),
      }),
    ];
  },
  addNodeView() {
    return solidNodeView(FieldRefChip);
  },
});

export interface EditorOptions {
  /** The block menu, opened by `/`. */
  slash?: Omit<SuggestOptions, "key" | "topLevelOnly" | "match">;
  /** Template editing: property blocks, and `{{name}}` with its menu. */
  template?: {
    slots: SlotContext;
    fieldMenu: Omit<SuggestOptions, "key" | "topLevelOnly" | "match">;
  };
  handle?: BlockHandleOptions;
  /** Pasted and dropped images: how to upload one, and what to do when that fails. */
  images?: Omit<ImageUploadOptions, "upload"> & { upload: (file: File) => Promise<string> };
}

export function editorExtensions(options: EditorOptions = {}): AnyExtension[] {
  const tools: AnyExtension[] = [MoveBlock, SteadyFocus, blockHandle(options.handle), EdgeDropColumns, ColumnRules];
  if (options.slash) tools.push(placeholder(options.template ? "Type / for blocks and properties, {{ for a field" : "Type / for blocks"));
  if (options.slash) {
    tools.push(suggestion({ key: "slashMenu", topLevelOnly: true, match: matchSlash, ...options.slash }));
  }
  if (options.template) {
    tools.push(templateSlot(options.template.slots), FieldRef);
    tools.push(
      suggestion({
        key: "fieldMenu",
        topLevelOnly: false,
        match: (before) => matchFieldRef(before),
        ...options.template.fieldMenu,
      }),
    );
  }
  return [
    ...tools,
    // The document holds layouts as well as blocks.
    VaultDoc,
    Columns,
    Column,
    StarterKit.configure({
      document: false,
      heading: { levels: [1, 2, 3] },
      code: false,
      codeBlock: false,
      horizontalRule: false,
      trailingNode: false,
      blockquote: {},
      link: { openOnClick: false, autolink: false, linkOnPaste: true },
    }),
    // Markup lets code sit inside bold, a link or a colour, so code excludes no other mark.
    Code.extend({ excludes: "" }),
    Highlight,
    Colour,
    TaskList,
    TaskItem.configure({ nested: true }),
    Mention,
    PlaceholderLink,
    MentionPickerNode,
    FollowLinks,
    VaultBlock,
    ImageUpload.configure(options.images ?? {}),
  ];
}

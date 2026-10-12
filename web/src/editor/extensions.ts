// The editor's schema: TipTap's text blocks and marks, plus TheVault's colour mark, mentions,
// the mention picker and a holder for content blocks this editor does not edit yet (D29, D34).

import { type AnyExtension, Mark, Node, mergeAttributes, nodeInputRule } from "@tiptap/core";
import Code from "@tiptap/extension-code";
import Highlight from "@tiptap/extension-highlight";
import { TaskItem, TaskList } from "@tiptap/extension-list";
import StarterKit from "@tiptap/starter-kit";
import { BlockHolder } from "./BlockHolder";
import { MentionChip } from "./MentionChip";
import { MentionPicker } from "./MentionPicker";
import { PALETTE } from "./markup";
import { solidNodeView } from "./solidNodeView";

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    colour: {
      setColour: (colour: string) => ReturnType;
      unsetColour: () => ReturnType;
    };
    mentionPicker: {
      insertMentionPicker: () => ReturnType;
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
 * A search box standing where a mention will go. Typing `[[` or pressing the toolbar's mention
 * button inserts one; choosing a note replaces it with a mention. It is never saved.
 */
export const MentionPickerNode = Node.create({
  name: "mentionPicker",
  group: "inline",
  inline: true,
  atom: true,
  selectable: false,
  parseHTML() {
    return [];
  },
  renderHTML() {
    return ["span", { "data-mention-picker": "" }];
  },
  addCommands() {
    return {
      insertMentionPicker:
        () =>
        ({ commands }) =>
          commands.insertContent({ type: this.name }),
    };
  },
  addInputRules() {
    return [nodeInputRule({ find: /\[\[$/, type: this.type })];
  },
  addNodeView() {
    return solidNodeView(MentionPicker);
  },
});

/** A content block this editor keeps but does not edit: an image, code, terminal and so on. */
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

export function editorExtensions(): AnyExtension[] {
  return [
    StarterKit.configure({
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
    MentionPickerNode,
    VaultBlock,
  ];
}

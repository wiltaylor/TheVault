// `toggle` and `admonition` content blocks (docs/vault-format.md, "toggle", "admonition"): the
// admonition presets, and whether a toggle (or a collapsible admonition) is open.
//
// Open or closed is never stored in the note. Each device remembers it in local storage, per
// note and field (the scope) and per block: content blocks have no ids, so a block is named by
// its place in the content, as `2` or `3.0`, the indexes from the top down.

import { createSignal } from "solid-js";
import type { Block } from "../api";

export const ADMONITION_KINDS = ["note", "info", "tip", "success", "warning", "danger", "question", "quote"] as const;
export type AdmonitionKind = (typeof ADMONITION_KINDS)[number];

export interface Preset {
  /** A palette colour (OQ-S4). */
  colour: string;
  /** An icon name, as `TypeIcon` draws them. */
  icon: string;
  /** The title shown when the block has none. */
  label: string;
}

/** Each kind's colour, icon and label; `schema::admonition_preset` holds the same. */
export const PRESETS: Record<AdmonitionKind, Preset> = {
  note: { colour: "gray", icon: "pencil", label: "Note" },
  info: { colour: "blue", icon: "info", label: "Info" },
  tip: { colour: "green", icon: "lightbulb", label: "Tip" },
  success: { colour: "green", icon: "circle-check", label: "Success" },
  warning: { colour: "orange", icon: "triangle-alert", label: "Warning" },
  danger: { colour: "red", icon: "octagon-alert", label: "Danger" },
  question: { colour: "purple", icon: "circle-question-mark", label: "Question" },
  quote: { colour: "brown", icon: "quote", label: "Quote" },
};

export function isAdmonitionKind(kind: unknown): kind is AdmonitionKind {
  return typeof kind === "string" && (ADMONITION_KINDS as readonly string[]).includes(kind);
}

/** The preset of `kind`, or the note preset for a kind this client does not know. */
export function presetOf(kind: unknown): Preset {
  return PRESETS[isAdmonitionKind(kind) ? kind : "note"];
}

/** What an admonition shows: its own colour, icon and title where it has them, else its kind's. */
export interface AdmonitionLook {
  kind: AdmonitionKind;
  colour: string;
  icon: string;
  /** The block's own title, markup, or "" when it has none. */
  title: string;
  /** The kind's label, shown when there is no title. */
  label: string;
  collapsible: boolean;
}

export function admonitionLook(fields: Record<string, unknown> | undefined): AdmonitionLook {
  const kind = isAdmonitionKind(fields?.kind) ? fields.kind : "note";
  const preset = PRESETS[kind];
  const str = (v: unknown) => (typeof v === "string" && v ? v : "");
  return {
    kind,
    colour: str(fields?.colour) || preset.colour,
    icon: str(fields?.icon) || preset.icon,
    title: str(fields?.title),
    label: preset.label,
    collapsible: fields?.collapsible === true,
  };
}

/** A new admonition of `kind`, with a line to type on. */
export function newAdmonition(kind: AdmonitionKind): Block {
  return { kind: "admonition", fields: { kind }, blocks: [{ kind: "paragraph", fields: { text: "" } }] };
}

// ---------------------------------------------------------------------------------------------
// Open or closed, per device

const KEY = "thevault.toggle";

function storageKey(scope: string, path: string): string {
  return `${KEY}:${scope}:${path}`;
}

/**
 * Whether the toggle at `path` in `scope` is open on this device, or `initial` when this device
 * has not opened or closed it. Without a scope (a note not yet saved) nothing is remembered.
 */
export function readOpen(scope: string | undefined, path: string, initial: boolean): boolean {
  if (!scope) return initial;
  try {
    const v = globalThis.localStorage?.getItem(storageKey(scope, path));
    return v === "1" ? true : v === "0" ? false : initial;
  } catch {
    return initial;
  }
}

/** Remember that the toggle at `path` in `scope` is open or closed on this device. */
export function writeOpen(scope: string | undefined, path: string, open: boolean) {
  if (!scope) return;
  try {
    globalThis.localStorage?.setItem(storageKey(scope, path), open ? "1" : "0");
  } catch {
    // Storage full or blocked: the state lasts as long as the page.
  }
}

// ---------------------------------------------------------------------------------------------
// Opening for a search

/**
 * The text a search opened a note with, and the note: a toggle in that note whose nested blocks
 * hold the text opens, so the hit is not hidden. The note page sets it from `?find=`.
 */
const [found, setFound] = createSignal<{ scope: string; text: string } | null>(null);

/** Set the search text note `noteId` was opened with, or clear it. */
export function setFindText(noteId: string, text: string | null) {
  setFound(text?.trim() ? { scope: noteId, text: text.trim().toLowerCase() } : null);
}

/** The search text, when `scope` (a note id, or a note id and field as `id/field`) is the note searched. */
export function findTextFor(scope: string | undefined): string | null {
  const f = found();
  if (!f || !scope) return null;
  return scope === f.scope || scope.startsWith(`${f.scope}/`) ? f.text : null;
}

/** Every string item of `blocks` and the blocks nested in them, lower-cased, one per line. */
export function nestedText(blocks: readonly Block[] | undefined): string {
  const out: string[] = [];
  const walk = (bs: readonly Block[]) => {
    for (const b of bs) {
      for (const v of Object.values(b.fields ?? {})) {
        if (typeof v === "string") out.push(v);
        // A table row's cells.
        else if (Array.isArray(v)) for (const c of v) if (typeof c === "string") out.push(c);
      }
      walk(b.blocks ?? []);
    }
  };
  walk(blocks ?? []);
  return out.join("\n").toLowerCase();
}

/** Whether a search for `find` (lower case) is a hit in `haystack`: every word of it is there. */
export function holdsFind(haystack: string, find: string | null): boolean {
  if (!find) return false;
  const words = find.split(/\s+/).filter(Boolean);
  return words.length > 0 && words.every((w) => haystack.includes(w));
}

/** A toggle's summary level, 0 for plain text. */
export function toggleLevel(block: Block): number {
  const n = Number(block.fields?.level ?? 0);
  return Number.isInteger(n) && n >= 0 && n <= 3 ? n : 0;
}

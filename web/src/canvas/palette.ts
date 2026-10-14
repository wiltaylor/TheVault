// The palette colours a canvas node, group or edge can take (OQ-S4), and how one is drawn. The
// hues come from the theme, as `--palette-<name>` in styles.css, so dark mode lightens them.

import { PALETTE } from "../editor/markup";

export type PaletteColour = (typeof PALETTE)[number];

export const PALETTE_COLOURS: readonly PaletteColour[] = PALETTE;

export function isPaletteColour(c: unknown): c is PaletteColour {
  return typeof c === "string" && (PALETTE as readonly string[]).includes(c);
}

/** The CSS colour of a palette name, or undefined for no colour or a name not in the palette. */
export function paletteCss(c: string | null | undefined): string | undefined {
  return isPaletteColour(c) ? `var(--palette-${c})` : undefined;
}

/** The `colour` item to store for a choice: the name, or undefined (no item) for no colour. */
export function colourItem(c: string | null | undefined): PaletteColour | undefined {
  return isPaletteColour(c) ? c : undefined;
}

/** A palette name as a person reads it: "Gray", "No colour". */
export function colourLabel(c: string | null | undefined): string {
  return isPaletteColour(c) ? c.charAt(0).toUpperCase() + c.slice(1) : "No colour";
}

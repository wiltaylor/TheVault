// Select options and their labels (ticket 103). A select field stores the option's value; its
// `option_labels`, index-aligned with `options`, give the text shown for each. An option with no
// label, or an empty one, shows its value.

import type { FieldDef } from "./api";

type Select = Pick<FieldDef, "options" | "option_labels">;

/** The text shown for option `value` of select field `f`. */
export function optionText(f: Select, value: string): string {
  const n = (f.options ?? []).indexOf(value);
  const label = n >= 0 ? f.option_labels?.[n] : undefined;
  return label ? label : value;
}

/** Every option of `f` as a value and the text shown for it, in display order. */
export function optionChoices(f: Select): { value: string; text: string }[] {
  return (f.options ?? []).map((value) => ({ value, text: optionText(f, value) }));
}

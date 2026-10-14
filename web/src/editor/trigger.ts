// When typing `@` opens the mention picker: only at the start of a word, so an email address or
// a handle in the middle of text stays text.

/** `@` just typed at the start of the text, or after a space or an opening bracket or quote. */
export const AT_TRIGGER = /(?:^|[\s([{"'“‘«<])@$/;

/** Whether typing `@` after `before` (the text before the caret in its block) opens the picker. */
export function atOpensPicker(before: string): boolean {
  return AT_TRIGGER.test(`${before}@`);
}

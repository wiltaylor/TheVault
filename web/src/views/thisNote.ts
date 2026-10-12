import { createContext, useContext } from "solid-js";

/** The note whose page, template or card holds the content being rendered, bound to `this`. */
export const ThisNote = createContext<string | undefined>(undefined);

export function useThisNote(): string | undefined {
  return useContext(ThisNote);
}

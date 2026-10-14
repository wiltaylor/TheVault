// Whether the browser holds a live session, and the types the signed-in app lays notes out with.

import { createSignal } from "solid-js";
import { ApiError, api, type Taxonomy, type TypeDef } from "./api";
import { onNoteWritten } from "./changes";

export type SessionState = "checking" | "in" | "out";

const [state, setState] = createSignal<SessionState>("checking");
const [types, setTypes] = createSignal<TypeDef[]>([]);
const [taxonomies, setTaxonomies] = createSignal<Taxonomy[]>([]);

export { state as sessionState, taxonomies, types };

/** The type named `name`, once the types have loaded. */
export function typeDef(name: string): TypeDef | undefined {
  return types().find((t) => t.name === name);
}

/** The taxonomy named `name`, once the taxonomies have loaded. */
export function taxonomyDef(name: string): Taxonomy | undefined {
  return taxonomies().find((t) => t.name === name);
}

/** Load the types and taxonomies, which also tells whether the session cookie is still good. */
export async function refreshSession(): Promise<void> {
  try {
    await reloadDefinitions();
    setState("in");
  } catch (e) {
    if (e instanceof ApiError && e.status === 401) {
      setState("out");
      return;
    }
    throw e;
  }
}

/** Load the types and taxonomies again, after an editor changed them. */
export async function reloadDefinitions(): Promise<void> {
  const [t, x] = await Promise.all([api.types(), api.taxonomies()]);
  setTypes(t);
  setTaxonomies([...x].sort((a, b) => a.name.localeCompare(b.name)));
}

// Terms picked for an open taxonomy that it did not hold. The save that holds them adds them on
// the server, so the next note write loads the taxonomies again to show them everywhere.
let termsPicked = false;

/** A new term was picked for an open taxonomy: load the taxonomies after the next note write. */
export function newTermPicked() {
  termsPicked = true;
}

onNoteWritten(() => {
  if (!termsPicked) return;
  termsPicked = false;
  api
    .taxonomies()
    .then((x) => setTaxonomies([...x].sort((a, b) => a.name.localeCompare(b.name))))
    .catch(() => undefined);
});

/** Log in. A wrong password rejects with the API's error. */
export async function logIn(password: string): Promise<void> {
  await api.login(password);
  await refreshSession();
}

export async function logOut(): Promise<void> {
  try {
    await api.logout();
  } finally {
    signedOut();
  }
}

/** Forget the session, after a logout or when a request answers 401. */
export function signedOut() {
  setState("out");
  setTypes([]);
  setTaxonomies([]);
}

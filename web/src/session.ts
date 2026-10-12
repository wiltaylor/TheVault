// Whether the browser holds a live session, and the types the signed-in app lays notes out with.

import { createSignal } from "solid-js";
import { ApiError, api, type TypeDef } from "./api";

export type SessionState = "checking" | "in" | "out";

const [state, setState] = createSignal<SessionState>("checking");
const [types, setTypes] = createSignal<TypeDef[]>([]);

export { state as sessionState, types };

/** The type named `name`, once the types have loaded. */
export function typeDef(name: string): TypeDef | undefined {
  return types().find((t) => t.name === name);
}

/** Load the types, which also tells whether the session cookie is still good. */
export async function refreshSession(): Promise<void> {
  try {
    setTypes(await api.types());
    setState("in");
  } catch (e) {
    if (e instanceof ApiError && e.status === 401) {
      setState("out");
      return;
    }
    throw e;
  }
}

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
}

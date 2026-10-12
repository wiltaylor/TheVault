import { A } from "@solidjs/router";
import { For, Show } from "solid-js";
import type { UsingNote } from "../api";

/** The notes a refused delete names, each a link to the note. */
export function UsingNotes(props: { notes: UsingNote[]; label: string }) {
  return (
    <Show when={props.notes.length > 0}>
      <ul class="using-notes" aria-label={props.label}>
        <For each={props.notes}>
          {(n) => (
            <li>
              <A href={`/notes/${n.id}`}>{n.title || n.id}</A> <span class="muted">{n.type}</span>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}

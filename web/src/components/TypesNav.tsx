// The left sidebar's Types section, below Pinned: a disclosure over every type, alphabetically,
// each opening its note list in the active pane (a middle-click opens a background tab).

import { For, Show, createMemo } from "solid-js";
import { types } from "../session";
import { setTypesOpen, sortTypesForNav, typeNavLabel, typesOpen } from "../sidebarTypes";
import { isActiveLink } from "../workspace/links";
import { TypeIcon } from "./Icon";

export function TypesNav(props: { onNavigate: () => void }) {
  const sorted = createMemo(() => sortTypesForNav(types()));
  return (
    <nav aria-label="Types" class="types-nav">
      <div class="types-nav-heading">
        <button
          type="button"
          class="types-nav-toggle"
          aria-expanded={typesOpen()}
          aria-controls="types-nav-list"
          onClick={() => setTypesOpen(!typesOpen())}
        >
          <span class="pin-chevron" classList={{ open: typesOpen() }} aria-hidden="true">
            <TypeIcon name="chevron-right" size={14} />
          </span>
          <span>Types</span>
        </button>
      </div>
      <Show when={typesOpen()}>
        <ul id="types-nav-list" class="types-nav-list">
          <For each={sorted()}>
            {(t) => (
              <li>
                <a
                  href={`/types/${t.name}`}
                  class="nav-link"
                  classList={{ active: isActiveLink(`/types/${t.name}`, true) }}
                  title={typeNavLabel(t)}
                  onClick={() => props.onNavigate()}
                >
                  <TypeIcon name={t.icon} size={16} />
                  <span>{typeNavLabel(t)}</span>
                </a>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </nav>
  );
}

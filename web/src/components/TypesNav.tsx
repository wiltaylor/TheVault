// The left sidebar's Types section, below Pinned: a disclosure over every type, alphabetically,
// each opening its note list in the active pane (a middle-click opens a background tab). Uploaded
// files sit in the same list as "Images and files", opening the assets page.

import { For, Show, createMemo } from "solid-js";
import { types } from "../session";
import { setTypesOpen, sortTypesForNav, typeNavLabel, typesOpen } from "../sidebarTypes";
import { isActiveLink } from "../workspace/links";
import { TypeIcon } from "./Icon";

/** The assets page's entry, listed among the types by its label. */
const ASSETS_ENTRY = { href: "/assets", label: "Images and files", icon: "images" };

export function TypesNav(props: { onNavigate: () => void }) {
  const sorted = createMemo(() => {
    const entries = sortTypesForNav(types()).map((t) => ({ href: `/types/${t.name}`, label: typeNavLabel(t), icon: t.icon }));
    const at = entries.findIndex((e) => e.label.localeCompare(ASSETS_ENTRY.label, undefined, { sensitivity: "base" }) > 0);
    entries.splice(at < 0 ? entries.length : at, 0, ASSETS_ENTRY);
    return entries;
  });
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
            {(e) => (
              <li>
                <a
                  href={e.href}
                  class="nav-link"
                  classList={{ active: isActiveLink(e.href, true) }}
                  title={e.label}
                  onClick={() => props.onNavigate()}
                >
                  <TypeIcon name={e.icon} size={16} />
                  <span>{e.label}</span>
                </a>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </nav>
  );
}

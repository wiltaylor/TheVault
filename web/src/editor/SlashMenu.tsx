import { For, Show, createEffect, on } from "solid-js";

/** Where the menu sits in the viewport, and how tall it may grow there. */
export interface MenuPos {
  top?: number;
  bottom?: number;
  left: number;
  maxHeight: number;
}

export interface MenuItem {
  id: string;
  label: string;
  /** The heading the item is listed under. */
  section: string;
  /** Extra words the filter matches. */
  keywords?: string;
  hint?: string;
  run: () => void;
}

/**
 * The menu a trigger typed in the editor opens: `/` lists blocks to insert, `{{` fields to show.
 * The editor keeps focus and drives it with the arrow keys, Enter and Escape; a click chooses.
 */
export function SuggestMenu(props: {
  label: string;
  items: MenuItem[];
  active: number;
  pos: MenuPos;
  onHover: (index: number) => void;
  onChoose: (item: MenuItem) => void;
}) {
  let list!: HTMLDivElement;
  createEffect(
    on(
      () => props.active,
      (i) => list?.querySelector(`[data-index="${i}"]`)?.scrollIntoView({ block: "nearest" }),
    ),
  );
  return (
    <div
      ref={list}
      class="suggest-menu"
      role="listbox"
      aria-label={props.label}
      style={{
        top: props.pos.top === undefined ? undefined : `${props.pos.top}px`,
        bottom: props.pos.bottom === undefined ? undefined : `${props.pos.bottom}px`,
        left: `${props.pos.left}px`,
        "max-height": `${props.pos.maxHeight}px`,
      }}
      // Keep the editor focused: the menu is driven from it.
      onMouseDown={(e) => e.preventDefault()}
    >
      <Show when={props.items.length > 0} fallback={<p class="suggest-empty muted">No matches</p>}>
        <For each={props.items}>
          {(item, i) => (
            <>
              <Show when={i() === 0 || props.items[i() - 1]?.section !== item.section}>
                <div class="suggest-section" role="presentation">
                  {item.section}
                </div>
              </Show>
              <div
                role="option"
                class="suggest-item"
                data-index={i()}
                aria-selected={i() === props.active}
                onMouseMove={() => props.onHover(i())}
                onClick={() => props.onChoose(item)}
              >
                <span>{item.label}</span>
                <Show when={item.hint}>
                  <span class="suggest-hint">{item.hint}</span>
                </Show>
              </div>
            </>
          )}
        </For>
      </Show>
    </div>
  );
}

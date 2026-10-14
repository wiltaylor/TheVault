import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { TypeIcon, iconNames } from "./Icon";

/** How many matches the grid shows at once; a search narrows the rest down. */
const SHOWN = 96;

/**
 * Choose a type's icon from the Lucide set: a button showing the current icon that opens a
 * searchable grid. `value` is the icon name stored in the type file, empty for the default icon.
 */
export function IconPicker(props: { id: string; value: string; onChange: (name: string) => void }) {
  const [open, setOpen] = createSignal(false);
  const [search, setSearch] = createSignal("");
  const [names] = createResource(open, () => iconNames());
  let root: HTMLDivElement | undefined;
  let searchBox: HTMLInputElement | undefined;

  const matches = createMemo(() => {
    const words = search().toLowerCase().trim().split(/[\s-]+/).filter(Boolean);
    const all = names() ?? [];
    if (words.length) return all.filter((n) => words.every((w) => n.includes(w)));
    // With nothing searched, the current icon comes first so it is always in view.
    return props.value && all.includes(props.value) ? [props.value, ...all.filter((n) => n !== props.value)] : all;
  });

  function choose(name: string) {
    props.onChange(name);
    setOpen(false);
    setSearch("");
  }

  function toggle() {
    setOpen(!open());
    if (open()) queueMicrotask(() => searchBox?.focus());
  }

  const onOutside = (e: PointerEvent) => {
    if (open() && root && !root.contains(e.target as Node)) setOpen(false);
  };
  onMount(() => document.addEventListener("pointerdown", onOutside));
  onCleanup(() => document.removeEventListener("pointerdown", onOutside));

  return (
    <div
      class="icon-picker"
      ref={root}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open()) {
          e.preventDefault();
          setOpen(false);
        }
      }}
    >
      <button
        type="button"
        id={props.id}
        class="icon-picker-button"
        aria-haspopup="dialog"
        aria-expanded={open()}
        onClick={toggle}
      >
        <TypeIcon name={props.value || null} />
        <span>{props.value || "Default icon"}</span>
      </button>
      <Show when={open()}>
        <div class="icon-picker-panel" role="dialog" aria-label="Choose an icon">
          <div class="icon-picker-search">
            <input
              ref={searchBox}
              type="search"
              placeholder="Search icons"
              aria-label="Search icons"
              value={search()}
              onInput={(e) => setSearch(e.currentTarget.value)}
            />
            <button type="button" onClick={() => choose("")} aria-pressed={!props.value}>
              No icon
            </button>
          </div>
          <Show when={!names.loading} fallback={<p class="muted">Loading icons…</p>}>
            <div class="icon-picker-grid" role="listbox" aria-label="Icons">
              <For each={matches().slice(0, SHOWN)}>
                {(name) => (
                  <button
                    type="button"
                    role="option"
                    aria-label={name}
                    aria-selected={props.value === name}
                    title={name}
                    onClick={() => choose(name)}
                  >
                    <TypeIcon name={name} size={22} />
                  </button>
                )}
              </For>
            </div>
            <p class="muted icon-picker-count">
              <Show
                when={matches().length > SHOWN}
                fallback={`${matches().length} ${matches().length === 1 ? "icon" : "icons"}`}
              >
                Showing {SHOWN} of {matches().length} icons. Search to narrow them down.
              </Show>
            </p>
          </Show>
        </div>
      </Show>
    </div>
  );
}

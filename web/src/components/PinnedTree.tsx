// The sidebar's Pinned section: the pinned tree of notes and folders, with a New folder button,
// a ⋯ menu per entry (rename, delete, unpin, Move to…), and drag and drop by pointer events, so it
// works with a mouse, a pen or a finger. Touch drags start from the grip, so the drawer still
// scrolls under a finger.

import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import type { PinItem } from "../api";
import { type DropZone, dropMove, findItem, folders, isWithin, placeOf } from "../pinTree";
import { createFolder, deleteFolder, isExpanded, movePin, pins, renameFolder, setExpandedFor, unpin } from "../pins";
import { typeDef } from "../session";
import { TypeIcon } from "./Icon";
import { isActiveLink } from "../workspace/links";

interface Drop {
  /** The entry dropped on, or null for the Pinned header (the end of the top level). */
  target: string | null;
  zone: DropZone;
}

function label(item: PinItem): string {
  return item.kind === "folder" ? item.name : item.title || "Untitled";
}

/** A one-line name field: Enter or leaving the field confirms, Escape cancels. */
export function NameInput(props: { value?: string; label: string; onDone: (name: string | null) => void }) {
  let input!: HTMLInputElement;
  let done = false;
  const finish = (name: string | null) => {
    if (done) return;
    done = true;
    props.onDone(name);
  };
  onMount(() => {
    input.focus();
    input.select();
  });
  return (
    <input
      ref={input}
      class="pin-name-input"
      aria-label={props.label}
      value={props.value ?? ""}
      onPointerDown={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          finish(e.currentTarget.value.trim() || null);
        } else if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          finish(null);
        }
      }}
      onBlur={(e) => finish(e.currentTarget.value.trim() || null)}
    />
  );
}

function Chevron(props: { open: boolean }) {
  return (
    <span class="pin-chevron" classList={{ open: props.open }} aria-hidden="true">
      <TypeIcon name="chevron-right" size={14} />
    </span>
  );
}

export function PinnedTree(props: { onNavigate: () => void }) {
  const [creating, setCreating] = createSignal(false);
  const [renaming, setRenaming] = createSignal<string | null>(null);
  const [menu, setMenu] = createSignal<{ id: string; moveTo: boolean } | null>(null);
  const [drag, setDrag] = createSignal<{ id: string; label: string; x: number; y: number } | null>(null);
  const [drop, setDrop] = createSignal<Drop | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  let nav!: HTMLElement;
  let suppressClick = false;

  // A drag ends with a click on whatever was under the pointer; that click must not navigate.
  onMount(() => {
    const swallow = (e: MouseEvent) => {
      if (!suppressClick) return;
      e.preventDefault();
      e.stopPropagation();
    };
    nav.addEventListener("click", swallow, true);
    onCleanup(() => nav.removeEventListener("click", swallow, true));
  });

  function report(p: Promise<unknown>) {
    setError(null);
    p.catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }

  function toggle(id: string) {
    setExpandedFor(id, !isExpanded(id));
  }

  function moveTo(id: string, parent: string | null) {
    setMenu(null);
    const count = parent === null ? pins().length : (findItem(pins(), parent)?.children.length ?? 0);
    if (parent !== null) setExpandedFor(parent, true);
    report(movePin(id, parent, count));
  }

  /** What a drop at (`x`, `y`) of `dragged` would do, or null when it would be refused. */
  function hitTest(x: number, y: number, dragged: string): Drop | null {
    const el = document.elementFromPoint(x, y);
    if (!(el instanceof Element) || !nav.contains(el)) return null;
    if (el.closest("[data-pin-root]")) {
      return dropMove(pins(), dragged, null, "inside") ? { target: null, zone: "inside" } : null;
    }
    const row = el.closest<HTMLElement>("[data-pin-id]");
    if (!row) return null;
    const id = row.dataset.pinId!;
    const r = row.getBoundingClientRect();
    const f = (y - r.top) / r.height;
    const zone: DropZone = f < 0.28 ? "before" : f > 0.72 ? "after" : "inside";
    return dropMove(pins(), dragged, id, zone) ? { target: id, zone } : null;
  }

  function startDrag(e: PointerEvent, item: PinItem) {
    if (e.button !== 0 || renaming() === item.id) return;
    const t = e.target as Element;
    if (t.closest(".pin-more, .pin-twisty, .pin-menu, input")) return;
    if (e.pointerType === "touch" && !t.closest(".pin-grip")) return;
    const { pointerId, clientX: startX, clientY: startY } = e;
    let active = false;

    const move = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      if (!active) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 5) return;
        active = true;
        setMenu(null);
        window.getSelection()?.removeAllRanges();
        document.body.classList.add("pin-dragging");
      }
      ev.preventDefault();
      setDrag({ id: item.id, label: label(item), x: ev.clientX, y: ev.clientY });
      setDrop(hitTest(ev.clientX, ev.clientY, item.id));
    };
    const finish = (commit: boolean) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", key, true);
      document.body.classList.remove("pin-dragging");
      if (!active) return;
      suppressClick = true;
      setTimeout(() => (suppressClick = false), 0);
      const d = commit ? drop() : null;
      setDrag(null);
      setDrop(null);
      if (!d) return;
      const to = dropMove(pins(), item.id, d.target, d.zone);
      if (!to) return;
      if (to.parent !== null) setExpandedFor(to.parent, true);
      report(movePin(item.id, to.parent, to.index));
    };
    const up = (ev: PointerEvent) => ev.pointerId === pointerId && finish(true);
    const cancel = (ev: PointerEvent) => ev.pointerId === pointerId && finish(false);
    const key = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape" || !active) return;
      ev.preventDefault();
      ev.stopPropagation();
      finish(false);
    };
    window.addEventListener("pointermove", move, { passive: false });
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("keydown", key, true);
  }

  function Menu(p: { item: PinItem; anchor: () => HTMLButtonElement | undefined }) {
    let el!: HTMLDivElement;
    const close = (refocus: boolean) => {
      setMenu(null);
      if (refocus) p.anchor()?.focus();
    };
    const items = () => [...el.querySelectorAll<HTMLElement>("[role=menuitem]")];
    onMount(() => {
      items()[0]?.focus();
      const outside = (e: PointerEvent) => {
        if (!el.contains(e.target as Node) && !p.anchor()?.contains(e.target as Node)) close(false);
      };
      document.addEventListener("pointerdown", outside, true);
      onCleanup(() => document.removeEventListener("pointerdown", outside, true));
    });
    function onKey(e: KeyboardEvent) {
      const list = items();
      const at = list.indexOf(document.activeElement as HTMLElement);
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close(true);
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const step = e.key === "ArrowDown" ? 1 : -1;
        list[(at + step + list.length) % list.length]?.focus();
      } else if (e.key === "Tab") {
        close(false);
      }
    }
    const place = () => placeOf(pins(), p.item.id);
    const targets = () => folders(pins()).filter((f) => !isWithin(pins(), p.item.id, f.id));
    return (
      <div
        ref={el}
        class="pin-menu"
        role="menu"
        aria-label={`Actions for ${label(p.item)}`}
        onKeyDown={onKey}
        onPointerDown={(e) => e.stopPropagation()}
      >
        <Show
          when={menu()?.moveTo}
          fallback={
            <>
              <Show when={p.item.kind === "folder"}>
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    setMenu(null);
                    setRenaming(p.item.id);
                  }}
                >
                  Rename
                </button>
              </Show>
              <button type="button" role="menuitem" onClick={() => setMenu({ id: p.item.id, moveTo: true })}>
                Move to…
              </button>
              <Show
                when={p.item.kind === "folder"}
                fallback={
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setMenu(null);
                      report(unpin(p.item.id));
                    }}
                  >
                    Unpin
                  </button>
                }
              >
                <button
                  type="button"
                  role="menuitem"
                  class="danger"
                  onClick={() => {
                    setMenu(null);
                    report(deleteFolder(p.item.id));
                  }}
                >
                  Delete folder
                </button>
              </Show>
            </>
          }
        >
          <p class="pin-menu-heading">Move to</p>
          <button
            type="button"
            role="menuitem"
            disabled={place()?.parent === null}
            onClick={() => moveTo(p.item.id, null)}
          >
            Top level
          </button>
          <For each={targets()}>
            {(f) => (
              <button
                type="button"
                role="menuitem"
                style={{ "padding-left": `${0.75 + f.depth * 0.75}rem` }}
                disabled={place()?.parent === f.id}
                onClick={() => moveTo(p.item.id, f.id)}
              >
                <TypeIcon name="folder" size={14} />
                <span>{f.name}</span>
              </button>
            )}
          </For>
        </Show>
      </div>
    );
  }

  function Entry(p: { item: PinItem; depth: number }) {
    let more: HTMLButtonElement | undefined;
    const open = () => isExpanded(p.item.id);
    const hasKids = () => p.item.children.length > 0;
    const zone = () => {
      const d = drop();
      return d && d.target === p.item.id ? d.zone : null;
    };
    return (
      <li
        class="pin-entry"
        classList={{
          "drop-before": zone() === "before",
          "drop-after": zone() === "after",
          dragging: drag()?.id === p.item.id,
        }}
        style={{ "--depth": p.depth }}
      >
        <div
          class="pin-row"
          classList={{ "drop-inside": zone() === "inside", "menu-open": menu()?.id === p.item.id }}
          data-pin-id={p.item.id}
          data-kind={p.item.kind}
          onPointerDown={(e) => startDrag(e, p.item)}
        >
          <span class="pin-grip" aria-hidden="true">
            <TypeIcon name="grip-vertical" size={14} />
          </span>
          <Show
            when={p.item.kind === "folder" && p.item}
            fallback={
              <>
                <Show when={hasKids()} fallback={<span class="pin-twisty-space" />}>
                  <button
                    type="button"
                    class="pin-twisty"
                    aria-label={`${open() ? "Collapse" : "Expand"} ${label(p.item)}`}
                    aria-expanded={open()}
                    onClick={() => toggle(p.item.id)}
                  >
                    <Chevron open={open()} />
                  </button>
                </Show>
                <a
                  href={`/notes/${p.item.id}`}
                  classList={{ active: isActiveLink(`/notes/${p.item.id}`, true) }}
                  class="pin-note"
                  title={label(p.item)}
                  draggable={false}
                  onClick={() => props.onNavigate()}
                >
                  <TypeIcon name={p.item.kind === "note" ? typeDef(p.item.type)?.icon : null} size={16} />
                  <span>{label(p.item)}</span>
                </a>
              </>
            }
          >
            <Show
              when={renaming() === p.item.id}
              fallback={
                <button
                  type="button"
                  class="pin-folder"
                  aria-expanded={hasKids() ? open() : undefined}
                  title={label(p.item)}
                  onClick={() => toggle(p.item.id)}
                  onDblClick={() => setRenaming(p.item.id)}
                >
                  <Chevron open={open() && hasKids()} />
                  <TypeIcon name={open() && hasKids() ? "folder-open" : "folder"} size={16} />
                  <span>{label(p.item)}</span>
                </button>
              }
            >
              <NameInput
                value={label(p.item)}
                label="Folder name"
                onDone={(name) => {
                  setRenaming(null);
                  if (name && name !== label(p.item)) report(renameFolder(p.item.id, name));
                }}
              />
            </Show>
          </Show>
          <button
            ref={more}
            type="button"
            class="pin-more"
            aria-label={`Actions for ${label(p.item)}`}
            aria-haspopup="menu"
            aria-expanded={menu()?.id === p.item.id}
            title="More actions"
            onClick={() => setMenu(menu()?.id === p.item.id ? null : { id: p.item.id, moveTo: false })}
          >
            <TypeIcon name="ellipsis" size={16} />
          </button>
          <Show when={menu()?.id === p.item.id}>
            <Menu item={p.item} anchor={() => more} />
          </Show>
        </div>
        <Show when={hasKids() && open()}>
          <ul>
            <For each={p.item.children}>{(c) => <Entry item={c} depth={p.depth + 1} />}</For>
          </ul>
        </Show>
      </li>
    );
  }

  return (
    <nav ref={nav} aria-label="Pinned" class="pinned" onDragStart={(e) => e.preventDefault()}>
      <div class="pinned-header" data-pin-root classList={{ "drop-inside": !!drop() && drop()!.target === null }}>
        <h2>Pinned</h2>
        <button
          type="button"
          class="pinned-add"
          aria-label="New folder"
          title="New folder"
          onClick={() => setCreating(true)}
        >
          <TypeIcon name="folder-plus" size={16} />
        </button>
      </div>
      <Show when={creating()}>
        <div class="pin-row pin-row-new">
          <span class="pin-twisty-space" />
          <TypeIcon name="folder" size={16} />
          <NameInput
            label="New folder name"
            onDone={(name) => {
              setCreating(false);
              if (name) report(createFolder(name));
            }}
          />
        </div>
      </Show>
      <ul class="pin-tree">
        <For each={pins()}>{(item) => <Entry item={item} depth={0} />}</For>
      </ul>
      <Show when={pins().length === 0 && !creating()}>
        <p class="pinned-empty">Pin a note from its page to keep it here.</p>
      </Show>
      <Show when={error()}>
        <p class="error pinned-error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={drag()}>
        {(d) => (
          <div class="pin-drag-ghost" style={{ left: `${d().x + 14}px`, top: `${d().y + 12}px` }} aria-hidden="true">
            {d().label}
          </div>
        )}
      </Show>
    </nav>
  );
}

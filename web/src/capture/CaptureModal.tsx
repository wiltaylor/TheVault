// The capture dialog: type a thought or add an image, from any page, and it is saved as a
// fleeting note in the inbox. Images come from the Add image button (which lets a phone use its
// camera), from pasting, or from dropping files on the dialog.

import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { api } from "../api";
import { upload } from "../blocks/MediaBlocks";
import { showToast } from "../components/Toasts";
import { knowTitle } from "../editor/titles";
import { errorText } from "../pages/TypeEditor";
import { typeDef } from "../session";
import { FLEETING, captureDoc, captureFields, hasContent } from "./model";
import { closeCapture } from "./store";

interface Attachment {
  file: File;
  url: string;
  /** The asset id, once uploaded, so a retry after an error does not upload it again. */
  asset?: string;
}

const FOCUSABLE = 'button:not([disabled]), textarea, input:not([type="hidden"]):not([hidden]), a[href], [tabindex]:not([tabindex="-1"])';

/** Image files among `files`, named when a paste left them nameless. */
function imagesOf(files: Iterable<File>): File[] {
  return [...files]
    .filter((f) => f.type.startsWith("image/"))
    .map((f, i) => (f.name ? f : new File([f], `pasted-${Date.now()}-${i}.${f.type.split("/")[1] || "png"}`, { type: f.type })));
}

export function CaptureModal() {
  const [text, setText] = createSignal("");
  const [images, setImages] = createSignal<Attachment[]>([]);
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [confirming, setConfirming] = createSignal(false);
  const [dragging, setDragging] = createSignal(false);
  const type = () => typeDef(FLEETING);
  const canAttach = () => !type() || captureFields(type()).image !== undefined;
  const empty = () => !hasContent(text(), images().length);
  const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  let dialog!: HTMLDivElement;
  let area!: HTMLTextAreaElement;
  let fileInput!: HTMLInputElement;

  function add(files: File[]) {
    if (!files.length) return;
    if (!canAttach()) {
      setError("The fleeting note type has no asset field to hold images.");
      return;
    }
    setError(null);
    setImages((list) => [...list, ...files.map((file) => ({ file, url: URL.createObjectURL(file) }))]);
  }

  function removeImage(a: Attachment) {
    URL.revokeObjectURL(a.url);
    setImages((list) => list.filter((x) => x !== a));
    area.focus();
  }

  function close() {
    for (const a of images()) URL.revokeObjectURL(a.url);
    closeCapture();
  }

  /** Close, asking first when there is something that would be lost. */
  function dismiss() {
    if (saving()) return;
    if (empty()) return close();
    setConfirming(true);
    queueMicrotask(() => dialog.querySelector<HTMLElement>(".capture-confirm button")?.focus());
  }

  async function save() {
    if (empty() || saving()) return;
    setSaving(true);
    setError(null);
    try {
      const ids: string[] = [];
      for (const a of images()) {
        if (!a.asset) a.asset = (await upload(a.file)).id;
        ids.push(a.asset);
      }
      const saved = await api.createNote(FLEETING, captureDoc(type(), text(), ids));
      const title = saved.note.fields.title;
      if (typeof title === "string") knowTitle(saved.id, title, saved.type);
      close();
      showToast("Saved to inbox", { label: "Open", href: `/notes/${saved.id}` });
    } catch (e) {
      setError(errorText(e));
      setSaving(false);
    }
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (confirming()) {
        setConfirming(false);
        area.focus();
      } else dismiss();
      return;
    }
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void save();
      return;
    }
    if (e.key === "Tab") {
      const items = [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null || el === document.activeElement);
      if (!items.length) return;
      const first = items[0]!;
      const last = items[items.length - 1]!;
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }

  function onPaste(e: ClipboardEvent) {
    const files = imagesOf([...(e.clipboardData?.items ?? [])].flatMap((i) => (i.kind === "file" ? [i.getAsFile()].filter((f): f is File => !!f) : [])));
    if (!files.length) return;
    e.preventDefault();
    add(files);
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    setDragging(false);
    add(imagesOf(e.dataTransfer?.files ?? []));
  }

  onMount(() => {
    area.focus();
    // Focus that wanders out of the dialog (a click on the page behind it) comes back.
    const keep = (e: FocusEvent) => {
      if (e.target instanceof Node && !dialog.contains(e.target)) area.focus();
    };
    document.addEventListener("focusin", keep);
    onCleanup(() => {
      document.removeEventListener("focusin", keep);
      // After the dialog goes, so that what opened it (the + button) shows again first.
      setTimeout(() => {
        if (returnTo?.isConnected) returnTo.focus();
      });
    });
  });

  return (
    <div
      class="capture-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) dismiss();
      }}
    >
      <div
        ref={dialog}
        class="capture-dialog"
        classList={{ dragging: dragging() }}
        role="dialog"
        aria-modal="true"
        aria-labelledby="capture-heading"
        onKeyDown={onKeyDown}
        onPaste={onPaste}
        onDragOver={(e) => {
          if ([...(e.dataTransfer?.types ?? [])].includes("Files")) {
            e.preventDefault();
            setDragging(true);
          }
        }}
        onDragLeave={(e) => {
          if (!dialog.contains(e.relatedTarget as Node | null)) setDragging(false);
        }}
        onDrop={onDrop}
      >
        <header class="capture-head">
          <h2 id="capture-heading">Capture</h2>
          <span class="muted capture-hint">Saved to your inbox</span>
          <button type="button" class="capture-x" aria-label="Close" title="Close (Esc)" onClick={dismiss}>
            ×
          </button>
        </header>
        <textarea
          ref={area}
          class="capture-text"
          aria-label="Capture text"
          placeholder="Type a thought, paste or drop an image…"
          rows={5}
          value={text()}
          disabled={saving()}
          onInput={(e) => {
            setText(e.currentTarget.value);
            setConfirming(false);
          }}
        />
        <Show when={images().length}>
          <ul class="capture-images" aria-label="Attached images">
            <For each={images()}>
              {(a, i) => (
                <li class="capture-image">
                  <img src={a.url} alt={a.file.name} />
                  <button
                    type="button"
                    class="capture-image-remove"
                    aria-label={`Remove image ${i() + 1}`}
                    title="Remove image"
                    disabled={saving()}
                    onClick={() => removeImage(a)}
                  >
                    ×
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <Show when={dragging()}>
          <p class="capture-drop muted" aria-hidden="true">
            Drop images to attach them
          </p>
        </Show>
        <Show when={error()}>
          <p class="error capture-error" role="alert">
            {error()}
          </p>
        </Show>
        <Show
          when={confirming()}
          fallback={
            <footer class="capture-actions">
              <Show when={canAttach()}>
                <input
                  ref={fileInput}
                  type="file"
                  accept="image/*"
                  multiple
                  hidden
                  onChange={(e) => {
                    add(imagesOf(e.currentTarget.files ?? []));
                    e.currentTarget.value = "";
                    area.focus();
                  }}
                />
                <button type="button" disabled={saving()} onClick={() => fileInput.click()}>
                  Add image
                </button>
              </Show>
              <span class="capture-spacer" />
              <span class="muted capture-keys">Ctrl+Enter</span>
              <button type="button" class="primary" disabled={empty() || saving()} onClick={() => void save()}>
                {saving() ? "Saving…" : "Save"}
              </button>
            </footer>
          }
        >
          <footer class="capture-actions capture-confirm" role="group" aria-label="Discard this capture?">
            <span>Discard this capture?</span>
            <span class="capture-spacer" />
            <button
              type="button"
              onClick={() => {
                setConfirming(false);
                area.focus();
              }}
            >
              Keep editing
            </button>
            <button type="button" class="danger" onClick={close}>
              Discard
            </button>
          </footer>
        </Show>
      </div>
    </div>
  );
}

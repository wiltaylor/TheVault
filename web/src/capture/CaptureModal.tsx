// The capture dialog: type a thought, add an image or a photo, or draw, from any page, and it is
// saved as a fleeting note in the inbox. Images come from the Add image button, from pasting, or
// from dropping files on the dialog; photos from the Camera button (capture K1); drawings from the
// Pen button (capture K2). Send to sends the saved capture to an agent (capture K3).

import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { newUlid } from "../agentCalls";
import { agentLabel, agents, refreshAgents } from "../agents";
import { type Block, api } from "../api";
import { upload } from "../blocks/MediaBlocks";
import { showToast } from "../components/Toasts";
import { knowTitle } from "../editor/titles";
import { errorText } from "../pages/TypeEditor";
import { typeDef } from "../session";
import { CameraPreview, type CameraMode, cameraMode } from "./Camera";
import { FLEETING, captureDoc, captureFields, hasContent } from "./model";
import { InkThumb, PenSurface } from "./Pen";
import { closeCapture, lastSendTo, rememberSendTo, sendToAgents } from "./store";

interface Attachment {
  file: File;
  url: string;
  /** The asset id, once uploaded, so a retry after an error does not upload it again. */
  asset?: string;
}

const FOCUSABLE =
  'button:not([disabled]), textarea, select, input:not([type="hidden"]):not([hidden]), a[href], [tabindex]:not([tabindex="-1"])';

/** Image files among `files`, named when a paste left them nameless. */
function imagesOf(files: Iterable<File>): File[] {
  return [...files]
    .filter((f) => f.type.startsWith("image/"))
    .map((f, i) => (f.name ? f : new File([f], `pasted-${Date.now()}-${i}.${f.type.split("/")[1] || "png"}`, { type: f.type })));
}

export function CaptureModal() {
  const [text, setText] = createSignal("");
  const [images, setImages] = createSignal<Attachment[]>([]);
  const [drawings, setDrawings] = createSignal<Block[]>([]);
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [confirming, setConfirming] = createSignal(false);
  const [dragging, setDragging] = createSignal(false);
  const [camera, setCamera] = createSignal<CameraMode>(null);
  const [previewing, setPreviewing] = createSignal(false);
  const [drawing, setDrawing] = createSignal(false);
  const [sendTo, setSendTo] = createSignal(lastSendTo());
  const type = () => typeDef(FLEETING);
  const canAttach = () => !type() || captureFields(type()).image !== undefined;
  const hasBody = () => captureFields(type()).body !== undefined;
  const empty = () => !hasContent(text(), images().length + drawings().length);
  const targets = createMemo(() => sendToAgents(agents()));
  // The remembered choice, while it is still in the list.
  const target = () => targets().find((a) => a.name === sendTo()) ?? null;
  const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  let dialog!: HTMLDivElement;
  let area!: HTMLTextAreaElement;
  let fileInput!: HTMLInputElement;
  let cameraInput: HTMLInputElement | undefined;

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

  function removeDrawing(d: Block) {
    setDrawings((list) => list.filter((x) => x !== d));
    area.focus();
  }

  function openCamera() {
    if (camera() === "capture") cameraInput?.click();
    else setPreviewing(true);
  }

  function closeCamera() {
    setPreviewing(false);
    area.focus();
  }

  function closePen() {
    setDrawing(false);
    queueMicrotask(() => area.focus());
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
      const agent = hasBody() ? target() : null;
      const call = agent ? { agent: agent.name, label: agentLabel(agent), call: newUlid() } : null;
      const saved = await api.createNote(FLEETING, captureDoc(type(), text(), ids, new Date(), { drawings: drawings(), call }));
      const title = saved.note.fields.title;
      if (typeof title === "string") knowTitle(saved.id, title, saved.type);
      close();
      const open = { label: "Open", href: `/notes/${saved.id}` };
      showToast(call ? `Saved and sent to ${call.label}` : "Saved to inbox", open);
    } catch (e) {
      setError(errorText(e));
      setSaving(false);
    }
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (drawing()) return;
      if (previewing()) closeCamera();
      else if (confirming()) {
        setConfirming(false);
        area.focus();
      } else dismiss();
      return;
    }
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      if (!drawing()) void save();
      return;
    }
    if (e.key === "Tab") {
      const scope = drawing() ? (dialog.querySelector<HTMLElement>(".capture-pen") ?? dialog) : dialog;
      const items = [...scope.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null || el === document.activeElement);
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
    if (drawing()) return;
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
    void cameraMode().then(setCamera);
    if (!agents().length) void refreshAgents();
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
        if (e.target === e.currentTarget && !drawing()) dismiss();
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
          if (!drawing() && [...(e.dataTransfer?.types ?? [])].includes("Files")) {
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
        <Show when={hasBody()}>
          <label class="capture-send-to">
            <span class="muted">Send to</span>
            <select
              value={target()?.name ?? ""}
              disabled={saving()}
              onChange={(e) => {
                setSendTo(e.currentTarget.value);
                rememberSendTo(e.currentTarget.value);
              }}
            >
              <option value="" selected={!target()}>
                No one
              </option>
              <For each={targets()}>
                {(a) => (
                  <option value={a.name} selected={target()?.name === a.name}>
                    {agentLabel(a)}
                  </option>
                )}
              </For>
            </select>
          </label>
        </Show>
        <Show when={previewing()}>
          <CameraPreview
            onClose={closeCamera}
            onUse={(photo) => {
              add([photo]);
              closeCamera();
            }}
          />
        </Show>
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
        <Show when={drawings().length}>
          <ul class="capture-images" aria-label="Drawings">
            <For each={drawings()}>
              {(d, i) => (
                <li class="capture-image capture-drawing">
                  <InkThumb block={d} label={`Drawing ${i() + 1}`} />
                  <button
                    type="button"
                    class="capture-image-remove"
                    aria-label={`Remove drawing ${i() + 1}`}
                    title="Remove drawing"
                    disabled={saving()}
                    onClick={() => removeDrawing(d)}
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
                <Show when={camera()}>
                  <Show when={camera() === "capture"}>
                    <input
                      ref={cameraInput}
                      type="file"
                      accept="image/*"
                      capture="environment"
                      hidden
                      data-testid="capture-camera-input"
                      onChange={(e) => {
                        add(imagesOf(e.currentTarget.files ?? []));
                        e.currentTarget.value = "";
                        area.focus();
                      }}
                    />
                  </Show>
                  <button type="button" disabled={saving() || previewing()} onClick={openCamera}>
                    Camera
                  </button>
                </Show>
              </Show>
              <Show when={hasBody()}>
                <button type="button" disabled={saving()} onClick={() => setDrawing(true)}>
                  Pen
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
        <Show when={drawing()}>
          <PenSurface
            onDiscard={closePen}
            onDone={(d) => {
              setDrawings((list) => [...list, d]);
              setConfirming(false);
              closePen();
            }}
          />
        </Show>
      </div>
    </div>
  );
}

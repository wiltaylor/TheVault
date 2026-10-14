// Brief messages at the bottom of the window, each with an optional action, such as "Saved to
// inbox" with Open, or "Marked done" with Undo. A toast goes after a few seconds or on its ×.

import { For, Show, createSignal } from "solid-js";

export interface ToastAction {
  label: string;
  /** A link the action follows, as any link in the app: a Ctrl/middle-click opens a background tab. */
  href?: string;
  run?: () => void;
}

interface Toast {
  id: number;
  text: string;
  action?: ToastAction;
}

const [toasts, setToasts] = createSignal<Toast[]>([]);
let next = 1;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

/** How long a toast stays, in milliseconds. */
export const TOAST_MS = 6000;

export function dismissToast(id: number) {
  clearTimeout(timers.get(id));
  timers.delete(id);
  setToasts((list) => list.filter((t) => t.id !== id));
}

/** Show `text` for a few seconds, with `action` beside it. Returns the toast's id. */
export function showToast(text: string, action?: ToastAction): number {
  const id = next++;
  setToasts((list) => [...list.slice(-2), { id, text, action }]);
  timers.set(
    id,
    setTimeout(() => dismissToast(id), TOAST_MS),
  );
  return id;
}

export function Toasts() {
  return (
    // A live region without the status role, so a page's own status line stays the only one.
    <div class="toasts" aria-live="polite">
      <For each={toasts()}>
        {(t) => (
          <div class="toast" data-toast={t.id}>
            <span class="toast-text">{t.text}</span>
            <Show when={t.action}>
              {(a) => (
                <Show
                  when={a().href}
                  fallback={
                    <button
                      type="button"
                      class="toast-action"
                      onClick={() => {
                        dismissToast(t.id);
                        a().run?.();
                      }}
                    >
                      {a().label}
                    </button>
                  }
                >
                  <a
                    href={a().href}
                    class="toast-action"
                    onClick={() => {
                      dismissToast(t.id);
                      a().run?.();
                    }}
                  >
                    {a().label}
                  </a>
                </Show>
              )}
            </Show>
            <button type="button" class="toast-close" aria-label="Dismiss" onClick={() => dismissToast(t.id)}>
              ×
            </button>
          </div>
        )}
      </For>
    </div>
  );
}

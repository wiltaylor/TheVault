import { Show, createSignal } from "solid-js";
import { type SentDeviceNotes, api } from "../api";
import { refreshSync } from "../sync/store";
import { errorText } from "./TypeEditor";

// Settings > Devices > This device, "Send this device's notes to the server" (ticket 150): queue
// every note the app's local vault holds that the server has never had, each as a create, then
// sync sends them with its progress. It only adds to the queue: no note on the device changes. A
// device paired before vaults had ids keeps notes pulled from an earlier vault that the server
// never received; its first round after the update queues them once by itself, and the button
// does it again whenever asked.

/** What the button says it did. */
export function sentText(r: SentDeviceNotes): string {
  if (r.queued === 0) return "The server already has every note on this device. Nothing was queued.";
  const notes = r.queued === 1 ? "1 note" : `${r.queued} notes`;
  const assets = r.assets === 0 ? "" : r.assets === 1 ? " and 1 file" : ` and ${r.assets} files`;
  const after = r.syncing ? " Sync is sending them now." : " The next sync sends them.";
  return `Queued ${notes}${assets} the server did not have.${after}`;
}

export function SendDeviceNotes() {
  const [busy, setBusy] = createSignal(false);
  const [done, setDone] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  async function send() {
    setBusy(true);
    setDone(null);
    setError(null);
    try {
      const r = await api.sendDeviceNotes();
      setDone(sentText(r));
      void refreshSync();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="send-device-notes">
      <p class="muted">
        Send the server every note on this device that it does not have, such as notes kept from before this device
        joined this vault. Notes the server already has are left alone, and nothing on this device changes.
      </p>
      <div class="actions">
        <button type="button" disabled={busy()} onClick={() => void send()}>
          {busy() ? "Checking…" : "Send this device's notes to the server"}
        </button>
      </div>
      <Show when={done()}>
        <p role="status">{done()}</p>
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </div>
  );
}

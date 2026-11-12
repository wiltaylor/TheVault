import { For, Show, createResource, createSignal } from "solid-js";
import { SafetyActions } from "./SafetyActions";
import { api, type HeldNote } from "../api";
import { errorText } from "../pages/TypeEditor";

/** Metadata is session-only. Never use a query or an agent to obtain held content. */
export async function loadHeld(): Promise<HeldNote[]> {
  const notes: HeldNote[] = [];
  let total = 1;
  while (notes.length < total) {
    const page = await api.heldAgentNotes(notes.length);
    notes.push(...page.notes);
    total = page.total;
    if (!page.notes.length) break;
  }
  return notes;
}

export function HeldInbox(props: { notes: HeldNote[]; onReleased: (id: string) => void }) {
  const [selected, setSelected] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");

  async function release(notes: HeldNote[]) {
    if (busy()) return;
    setBusy(true);
    setError("");
    try {
      for (const note of notes) {
        await api.releaseAgentSafety(note.id, note.agent_safety.fingerprint);
        setSelected((ids) => ids.filter((id) => id !== note.id));
        props.onReleased(note.id);
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return <section aria-label="Held items">
    <p>Held from agents until you release them. Release puts items into the Inbox and runs their matching agent triggers.</p>
    <Show when={error()}><p role="alert" class="error">{error()}</p></Show>
    <SafetyActions selected={selected()} onReleased={(id) => { setSelected((ids) => ids.filter((n) => n !== id)); props.onReleased(id); }} />
    <Show when={props.notes.length} fallback={<p>No held items in this view.</p>}>
      <ul class="inbox-list"><For each={props.notes}>{(held) => {
        const [loadError, setLoadError] = createSignal("");
        const [note] = createResource(() => held.id, async (id) => {
          try { return await api.note(id); }
          catch (e) { setLoadError(errorText(e)); return null; }
        });
        const fields = () => note()?.note.fields ?? {};
        const title = () => String(fields().title ?? held.id);
        return <li class="inbox-item">
          <input type="checkbox" aria-label={`Select ${title()}`} disabled={busy()} checked={selected().includes(held.id)} onChange={(e) => setSelected((ids) => e.currentTarget.checked ? [...ids, held.id] : ids.filter((id) => id !== held.id))} />
          <div class="inbox-main">
            <a href={`/notes/${held.id}`}>{title()}</a>
            <p>{String(fields().from_raw ?? "Sender unavailable")}</p>
            <p>Reason: {held.agent_safety.reason}</p>
            <p>Scam probability: {typeof fields().scam_probability === "number" ? `${Math.round(Number(fields().scam_probability) * 100)}%` : "Unavailable"}</p>
            <Show when={loadError()}><p role="alert">{loadError()}</p></Show>
          </div>
          <button type="button" disabled={busy() || !note() || note()?.agent_safety?.fingerprint !== held.agent_safety.fingerprint} onClick={() => void release([held])}>Release</button>
        </li>;
      }}</For></ul>
    </Show>
  </section>;
}

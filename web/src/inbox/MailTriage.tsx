import { Show, createSignal } from "solid-js";
import { api } from "../api";
import { errorText } from "../pages/TypeEditor";

/** No Apply action: the result is a textual recommendation in the agent's run thread. */
export function MailTriage() {
  const [agent, setAgent] = createSignal("");
  const [limit, setLimit] = createSignal(20);
  const [preview, setPreview] = createSignal<Awaited<ReturnType<typeof api.mailTriagePreview>>>();
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  async function act(confirm: boolean) {
    setBusy(true); setMessage("");
    try {
      if (confirm) {
        const p = preview();
        setPreview(undefined);
        if (!p) return;
        const result = await api.mailTriageConfirm(agent(), p.preview);
        setMessage(result.queued ? "Queued. Read the recommendation in the agent's Runs page. No notes will be changed." : "Queue is full. Preview again later.");
      } else {
        setPreview(undefined);
        setPreview(await api.mailTriagePreview(agent(), limit()));
      }
    } catch (e) { setMessage(errorText(e)); }
    finally { setBusy(false); }
  }
  return <details class="mail-triage">
    <summary>Mail backlog recommendations</summary>
    <p>One read-only recommendation for 20–50 unprocessed emails or messages. No Apply action, changes, scripts or subagents. Model calls use the selected agent's account. Held notes are excluded: classify or release them in Held Inbox first.</p>
    <label>Agent name <input disabled={busy()} value={agent()} onInput={e => { setAgent(e.currentTarget.value); setPreview(undefined); }} /></label>
    <label>Batch size <input disabled={busy()} type="number" min="20" max="50" value={limit()} onInput={e => { setLimit(Number(e.currentTarget.value)); setPreview(undefined); }} /></label>
    <button disabled={busy() || !agent()} onClick={() => void act(false)}>Preview recommendation batch</button>
    <Show when={preview()}>{p => <p>{p().selected} selected from {p().count} eligible messages. Preview makes no model calls. <button disabled={busy()} onClick={() => void act(true)}>Queue {p().selected} recommendations as one summary</button></p>}</Show>
    <p role="status">{message()}</p>
    <Show when={agent()}><a href={`/agents/${encodeURIComponent(agent())}/triggers`}>Agent runs</a></Show>
  </details>;
}

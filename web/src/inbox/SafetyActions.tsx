import { For, Show, createSignal } from "solid-js";
import { api, type SafetyBackfillProgress } from "../api";
import { errorText } from "../pages/TypeEditor";

/** Only password sessions can preview or approve these server-owned actions. */
export function SafetyActions(props: { selected: string[]; onReleased: (id: string) => void }) {
  const [price, setPrice] = createSignal("");
  const [preview, setPreview] = createSignal<Awaited<ReturnType<typeof api.safetyBackfillPreview>>>();
  const [progress, setProgress] = createSignal<SafetyBackfillProgress>();
  const [release, setRelease] = createSignal<Awaited<ReturnType<typeof api.safetyReleasePreview>>>();
  const [kind, setKind] = createSignal("");
  const [reason, setReason] = createSignal("");
  const [since, setSince] = createSignal("");
  const [until, setUntil] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [result, setResult] = createSignal("");
  async function act(action: () => Promise<void>) {
    if (busy()) return;
    setBusy(true); setError("");
    try { await action(); } catch (e) { setError(errorText(e)); setPreview(undefined); setRelease(undefined); }
    finally { setBusy(false); }
  }
  function filter() {
    return { type: kind() || undefined, reason: reason() || undefined, since: since() ? new Date(since()).toISOString() : undefined, until: until() ? new Date(`${until()}T23:59:59.999Z`).toISOString() : undefined };
  }
  return <section aria-label="Historical safety actions">
    <h3>One-time safety pass</h3>
    <p>Classify held pre-existing email, message, call and event notes created in the last 90 days. Older notes stay held. One attempt per note, at most one request per second, no automatic retry. Cleared backlog notes do not start agents.</p>
    <label>Owner-entered estimated USD per classifier decision <input type="number" min="0" step="any" value={price()} onInput={(e) => { setPrice(e.currentTarget.value); setPreview(undefined); }} /></label>
    <p>This is an estimate, not verified provider pricing or a billing guarantee. No default price.</p>
    <button disabled={busy() || price().trim() === "" || !Number.isFinite(Number(price())) || Number(price()) < 0} onClick={() => void act(async () => setPreview(await api.safetyBackfillPreview(Number(price()))))}>Preview safety pass</button>
    <Show when={preview()}>{(p) => <div role="group" aria-label="Approve safety pass">
      <p>{p().count} notes × {p().decisions_per_note} decisions per note × ${p().unit_price} = estimated ${p().estimated_usd} USD</p>
      <p>Account: {p().account}. Model: {p().model}. {p().disclaimer}</p>
      <button disabled={busy()} onClick={() => void act(async () => { setProgress(await api.safetyBackfillApprove(p().preview)); setPreview(undefined); })}>Approve one-time safety pass</button>
      <button disabled={busy()} onClick={() => setPreview(undefined)}>Cancel safety pass</button>
    </div>}</Show>
    <button disabled={busy()} onClick={() => void act(async () => setProgress(await api.safetyBackfillProgress()))}>Refresh safety progress</button>
    <Show when={progress()}>{(p) => <div role="status">
      <p>Safety pass: {p().status}. Attempted {p().attempted ?? 0} / {p().count ?? 0}. Allowed {p().allowed ?? 0}. Failed {p().failed ?? 0}.</p>
      <For each={Object.entries(p().held ?? {})}>{([reason, count]) => <p>Held: {reason}: {count}</p>}</For>
    </div>}</Show>
    <h3>Bulk release held notes</h3>
    <p>Release makes notes available to agents. Matching triggers may run for notes whose arrival has not been delivered.</p>
    <label>Release type <select value={kind()} onChange={(e) => { setKind(e.currentTarget.value); setRelease(undefined); }}><option value="">All types</option><For each={["email", "message", "call", "event", "event_series", "email_thread", "conversation"]}>{(type) => <option>{type}</option>}</For></select></label>
    <label>Release reason <input value={reason()} onInput={(e) => { setReason(e.currentTarget.value); setRelease(undefined); }} /></label>
    <label>Created since <input type="date" value={since()} onInput={(e) => { setSince(e.currentTarget.value); setRelease(undefined); }} /></label>
    <label>Created until <input type="date" value={until()} onInput={(e) => { setUntil(e.currentTarget.value); setRelease(undefined); }} /></label>
    <button disabled={busy()} onClick={() => void act(async () => setRelease(await api.safetyReleasePreview(filter())))}>Preview filtered release</button>
    <button disabled={busy() || !props.selected.length} onClick={() => void act(async () => setRelease(await api.safetyReleasePreview({ ids: [...props.selected] })))}>Release selected</button>
    <Show when={release()}>{(p) => <div role="group" aria-label="Confirm release">
      <p>Release {p().count} held notes? Matching agent triggers may run.</p>
      <button disabled={busy()} onClick={() => void act(async () => {
        const result = await api.safetyReleaseApprove(p().preview);
        for (const id of result.released_ids) props.onReleased(id);
        setResult(`Released ${result.released}. Failed ${result.failed}.`);
        setRelease(undefined);
      })}>Confirm release</button>
      <button disabled={busy()} onClick={() => setRelease(undefined)}>Cancel release</button>
    </div>}</Show>
    <Show when={result()}><p role="status">{result()}</p></Show>
    <Show when={error()}><p role="alert" class="error">{error()}</p></Show>
  </section>;
}

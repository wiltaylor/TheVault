import { For, Show, createSignal } from "solid-js";
import { api, type ScamHistoryPreview, type ScamHistoryProgress } from "../api";
import { errorText } from "./TypeEditor";

export function ScamHistory() {
  const [days, setDays] = createSignal("30");
  const [price, setPrice] = createSignal("");
  const [preview, setPreview] = createSignal<ScamHistoryPreview>();
  const [progress, setProgress] = createSignal<ScamHistoryProgress>();
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  function invalidate() { setPreview(undefined); }
  async function action(fn: () => Promise<unknown>) {
    setBusy(true); setError("");
    try { await fn(); } catch (e) { invalidate(); setError(errorText(e)); }
    finally { setBusy(false); }
  }
  const valid = () => Number.isInteger(Number(days())) && Number(days()) >= 1 && Number(days()) <= 90
    && price().trim() !== "" && Number.isFinite(Number(price())) && Number(price()) >= 0;
  return <details aria-label="Historical scam run">
    <summary>Historical scam run</summary>
    <p>Incoming email and messages with missing output only, selected by sent date over 1–90 days. Existing verdicts are not replaced. No safety retry or replay of the one-time safety pass.</p>
    <p>Configured scam fields (normally title, sender, recipients, snippet and body) go to the external account and model shown in the preview. No arrival events are replayed. One sequential attempt per item, no automatic retry. Interrupted attempts remain visible and only pending items resume after restart.</p>
    <label>Historical days<input type="number" min="1" max="90" step="1" value={days()} disabled={busy()}
      onInput={(e) => { setDays(e.currentTarget.value); invalidate(); }} /></label>
    <label>Estimated USD per decision<input type="number" min="0" step="any" value={price()} disabled={busy()}
      onInput={(e) => { setPrice(e.currentTarget.value); invalidate(); }} /></label>
    <p>Owner-entered estimate, not verified provider pricing or a billing guarantee. There is no default price. Explicit zero is allowed.</p>
    <button disabled={busy() || !valid()} onClick={() => void action(async () => {
      setPreview(await api.scamHistoryPreview(Number(days()), Number(price())));
    })}>Preview historical scam run</button>
    <Show when={preview()}>{(p) => <section aria-label="Scam run preview">
      <p>{p().count} items in {p().days} days through {new Date(p().until_ms).toISOString()}. Account: {p().account}. Model: {p().model}.</p>
      <p>{p().decisions_per_item} decisions per item × ${p().unit_price} per decision. Estimated total: ${p().estimated_usd} USD.</p>
      <p>Approval fingerprint: <code>{p().fingerprint}</code>. Expires after 10 minutes. Any changed input requires a new preview.</p>
      <button disabled={busy()} onClick={() => void action(async () => {
        const shown = p();
        setProgress(await api.scamHistoryApprove(shown.fingerprint, shown.days, shown.unit_price)); invalidate();
      })}>Approve historical scam run</button>
    </section>}</Show>
    <button disabled={busy()} onClick={() => void action(async () => setProgress(await api.scamHistoryProgress()))}>Refresh scam run progress</button>
    <Show when={progress()}>{(p) => <section aria-label="Scam run progress">
      <p>Status: {p().status}. Items: {p().count ?? 0}. Durable records: {p().attempts.length}.</p>
      <For each={p().attempts}>{(a) => <p>{a.id}: {a.outcome}</p>}</For>
    </section>}</Show>
    <Show when={error()}><p role="alert">{error()}</p></Show>
  </details>;
}

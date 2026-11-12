import { For, Show, createSignal } from "solid-js";
import { api, type ApprovalPreview, type ApprovalResult, type StoredProposal } from "../api";
import { errorText } from "../pages/TypeEditor";

/** Explicit, current-thread selection. Nothing here creates an enduring approval rule. */
export function ApplyKind(props: { agent: string; thread: string }) {
  const [proposals, setProposals] = createSignal<StoredProposal[]>([]);
  const [preview, setPreview] = createSignal<ApprovalPreview>();
  const [results, setResults] = createSignal<ApprovalResult[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  async function act(f: () => Promise<void>) {
    if (busy()) return;
    setBusy(true);
    setError("");
    try { await f(); } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  async function load() {
    const thread = await api.agentThread(props.agent, props.thread);
    setProposals((thread.proposals ?? []).filter((p) => p.status === "pending" || p.status === "failed"));
    setPreview(undefined);
    setResults([]);
  }
  async function select(id: string) {
    setPreview(undefined);
    setResults([]);
    setPreview(await api.aiApplyKindPreview(props.thread, id));
  }
  return <div class="apply-kind">
    <button type="button" disabled={busy()} onClick={() => void act(load)}>Review by kind</button>
    <Show when={!preview()}>
      <For each={proposals()}>{(p) => <button type="button" disabled={busy()}
        onClick={() => void act(() => select(String(p.data.id)))}>
        Preview this kind: {p.event} / {String(p.data.op ?? "update")} / {String(p.data.type ?? p.data.note_id ?? p.data.id)}
      </button>}</For>
    </Show>
    <Show when={preview()}>{(p) => <section aria-label="Apply this kind preview">
      <p>{p().count} proposals: {p().kind.filter(Boolean).join(" / ")}. Only these IDs will be applied. Future proposals still need review.</p>
      <ul><For each={p().items}>{(item) => <li><code>{item.id}</code>: {item.summary}
        <details><summary>Proposed change</summary><pre>{JSON.stringify(item.data, null, 2)}</pre></details>
      </li>}</For></ul>
      <button type="button" disabled={busy() || (results().length > 0 && results().every((r) => r.ok))}
        onClick={() => void act(async () => { setResults((await api.aiApplyKindConfirm(p().preview)).results); })}>
        {results().length ? "Retry unsuccessful proposals" : `Confirm Apply ${p().count} proposals`}
      </button>
      <button type="button" disabled={busy()} onClick={() => { setPreview(undefined); setProposals([]); setResults([]); }}>Close preview</button>
      <ul aria-live="polite"><For each={results()}>{(r) => <li>{r.id}: {r.ok ? "Applied" : r.error?.message ?? "Failed"}</li>}</For></ul>
    </section>}</Show>
    <Show when={error()}><p role="alert">{error()}</p></Show>
  </div>;
}

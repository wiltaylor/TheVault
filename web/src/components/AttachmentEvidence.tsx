import { createEffect, createSignal, For, Show } from "solid-js";
import { api, type SafetyEvidence } from "../api";

export function AttachmentEvidence(props: { id: string; fingerprint?: string }) {
  const [evidence, setEvidence] = createSignal<SafetyEvidence>();
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  createEffect(() => {
    void props.id;
    void props.fingerprint;
    setEvidence(undefined);
    setError("");
  });
  async function load() {
    const id = props.id;
    const fingerprint = props.fingerprint;
    setBusy(true);
    setEvidence(undefined);
    setError("");
    try {
      const result = await api.safetyEvidence(id);
      if (props.id === id && props.fingerprint === fingerprint) setEvidence(result);
    }
    catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }
  return <section aria-label="Attachment classification evidence">
    <button type="button" disabled={busy()} onClick={() => void load()}>Review attachment evidence</button>
    <Show when={error()}><p role="alert">{error()}</p></Show>
    <Show when={evidence()}>{(data) => <>
      <p>{data().decision_current ? "Current decision matches this envelope." : "Stale or missing envelope decision. This is current content, not evidence of what Jev previously saw. Historical representations are not retained."}</p>
      <p>Stored decision: {data().decision_status ?? "none"}. A user release is not a Jev classification.</p>
      <p>Envelope v{data().envelope.envelope_version}: {data().bytes} / {data().limit} bytes (128 KiB aggregate cap).</p>
      <p>Estimated input: {data().estimated_input_tokens} tokens, payload only at 4 bytes/token, excluding prompt and output. Prices are unknown. Actual usage is not recorded yet (TV-217).</p>
      <Show when={data().bytes > data().limit}><p>Over cap: no envelope is sent to Jev and no attachment text is cleared.</p></Show>
      <p>Original HTML, binary files and images stay withheld from agents. Only extracted attachment text can be cleared.</p>
      <p>Showing evidence does not grant access. Every attachment must be ready, and every incoming owner must have current clearance.</p>
      <For each={data().omissions}>{(item) => <p>{item.id}: {item.reason}</p>}</For>
      <For each={data().envelope.attachments}>{(asset) => <article>
        <strong>{asset.filename ?? asset.id}</strong>
        <p>{asset.status === "ready" ? "Extracted text representation in current envelope" : `Text omitted: ${asset.status}`}</p>
        <pre>{JSON.stringify({ ...asset, text: undefined }, null, 2)}</pre>
        <Show when={asset.status === "ready"}><pre>{asset.text}</pre></Show>
      </article>}</For>
    </>}</Show>
  </section>;
}

import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import { api, type AgentSafety } from "../api";

const reasons: Record<string, string> = {
  awaiting_decision: "Waiting for the safety decision.",
  prompt_injection: "Possible instructions aimed at an AI agent.",
  sensitive_credentials: "May contain credentials or a login code.",
  scam: "Possible scam or phishing.",
  spam: "Unsolicited junk.",
  marketing: "Marketing content.",
  decision_failed: "The safety decision failed. Check the AI account settings.",
  invalid_decision: "No valid safety decision was returned.",
  content_changed: "The content changed after its safety decision.",
  predates_activation: "This note predates the gate and has not been reviewed.",
  unclassified: "This note has not been queued for a safety decision.",
  too_large: "This note exceeds the safety decision size limit (128 KiB).",
};

export function AgentSafetyBanner(props: { id: string; safety?: AgentSafety }) {
  const [safety, setSafety] = createSignal(props.safety);
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  createEffect(() => setSafety(props.safety));
  const held = () => ["held", "pending"].includes(safety()?.status ?? "");
  async function refresh() {
    try { setSafety((await api.note(props.id)).agent_safety); }
    catch (e) { setError(String(e)); }
  }
  createEffect(() => {
    if (safety()?.status !== "pending") return;
    const timer = setTimeout(() => void refresh(), 1500);
    onCleanup(() => clearTimeout(timer));
  });
  async function release() {
    const current = safety();
    if (!current || busy()) return;
    setBusy(true);
    setError("");
    try { setSafety((await api.releaseAgentSafety(props.id, current.fingerprint)).agent_safety); }
    catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }
  return <Show when={held()}>
    <aside role="status" class="agent-safety-banner">
      <strong>Held from agents</strong>
      <p>{reasons[safety()?.reason ?? ""] ?? "This note has no valid safety clearance."} You can still read and edit it.</p>
      <p>Release only after reviewing this note. {safety()?.reason === "predates_activation"
        ? "Agents may then read it. Arrival triggers are not replayed for existing notes."
        : "Agents may then read it and run matching arrival triggers."}</p>
      <button type="button" disabled={busy()} onClick={() => void release()}>Release reviewed note to agents</button>
      <button type="button" disabled={busy()} onClick={() => void refresh()}>Refresh safety status</button>
      <Show when={error()}><p role="alert">{error()}</p></Show>
    </aside>
  </Show>;
}

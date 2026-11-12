import { Show } from "solid-js";
import { type Part, useChatSession } from "./store";

export function SecretCard(props: { part: Extract<Part, { kind: "browser_secret" }>; turn: number; index: number }) {
  const session = useChatSession();
  const p = () => props.part.proposal;
  const open = () => ["open", "failed", "applying"].includes(props.part.state);
  return <section class="ai-proposal" aria-label="Proposed secret fill">
    <p>Enter <strong>{p().secret}</strong> ({p().field}) at <strong>{p().origin}</strong></p>
    <p>Target: <code>{p().target}</code>. Session: <code>{p().session}</code>.</p>
    <Show when={p().unrestricted_origin}><p role="note">Warning: this secret has no origin restrictions and may be entered on any site.</p></Show>
    <Show when={props.part.error}><p role="alert">{props.part.error}</p></Show>
    <Show when={open()} fallback={<p role="status">{props.part.state === "applied" ? "Filled." : "Discarded."}</p>}>
      <p class="muted">Only an owner session or admin user token can apply this fill. The value is never shown here.</p>
      <button type="button" disabled={props.part.state === "applying"} onClick={() => void session.applySecret(props.turn, props.index)}>Apply</button>
      <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardSecret(props.turn, props.index)}>Discard</button>
    </Show>
  </section>;
}

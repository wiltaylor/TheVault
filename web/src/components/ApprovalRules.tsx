import { For, Show, createSignal } from "solid-js";
import type { AgentApprovalRule, AgentPermissions } from "../api";

/** Presentation only. The server validates and enforces permissions and quota. */
export function ApprovalRules(p: { permissions: AgentPermissions; editable: boolean; set: (rules: AgentApprovalRule[]) => void }) {
  const [type, setType] = createSignal("");
  const [fields, setFields] = createSignal("");
  const [cap, setCap] = createSignal("0");
  const [error, setError] = createSignal("");
  const rules = () => p.permissions.approval_rules ?? [];
  const outsideChangePermissions = (r: AgentApprovalRule) => {
    const types = p.permissions.change_types ?? p.permissions.types;
    const fields = p.permissions.change_fields?.[r.note_type];
    return (types != null && !types.includes(r.note_type)) ||
      (fields != null && r.fields.some((field) => !fields.includes(field)));
  };
  function add() {
    const names = fields().split(",").map((s) => s.trim()).filter(Boolean);
    const name = /^[a-z][a-z0-9_]*$/;
    const n = Number(cap());
    if (!name.test(type().trim()) || !names.length || names.some((s) => !name.test(s)) || !/^\d+$/.test(cap()) || !Number.isSafeInteger(n) || n < 0) {
      setError("Use a type name, comma-separated field names and a nonnegative safe integer cap.");
      return;
    }
    p.set([...rules(), { operation: "update", note_type: type().trim(), fields: [...new Set(names)], daily_cap: n }]);
    setError("");
  }
  return <fieldset class="agent-fieldset">
    <h2>Automatic approval rules</h2>
    <p>Only explicit field updates can be approved automatically. Zero means no allowance. Caps count attempted updates per UTC day. An attempt is released only when the write provably did not happen. Uncertain attempts stay reserved across restarts and UTC days. Saving requires an owner session.</p>
    <p>Effective limits: rules never expand read types, change types, change fields, caller scope or tool permissions. A rule also approves its matching update in unattended runs started by incoming mail. In unattended runs, a change without a matching rule is refused or held, not always a proposal. Calendar, Outbox and protected-code exclusions apply. The outside-text action guard is not yet in place (TV-245). Delete and archive rules never approve changes automatically.</p>
    <p>Read types: {p.permissions.types?.join(", ") ?? "all"}. Change types: {p.permissions.change_types?.join(", ") ?? "read types"}. Change fields: {JSON.stringify(p.permissions.change_fields ?? {})} (unlisted types have no field restriction).</p>
    <For each={rules()}>{(r, i) => <div>
      <span>{r.operation} {r.note_type}: {r.fields.join(", ")} — {r.daily_cap}/UTC day{r.operation !== "update" ? " (held, never automatic)" : outsideChangePermissions(r) ? " (never applies (outside this agent's change permissions))" : r.daily_cap === 0 ? " (no allowance)" : " (subject to server limits)"}</span>
      <Show when={p.editable}><button type="button" onClick={() => p.set(rules().filter((_, index) => index !== i()))}>Remove rule {i() + 1}</button></Show>
    </div>}</For>
    <Show when={p.editable} fallback={<p>Open the web app with an owner session to edit rules.</p>}>
      <label>Rule type<input value={type()} onInput={(e) => setType(e.currentTarget.value)} /></label>
      <label>Rule fields<input value={fields()} onInput={(e) => setFields(e.currentTarget.value)} placeholder="processed" /></label>
      <label>Rule daily cap<input value={cap()} onInput={(e) => setCap(e.currentTarget.value)} inputmode="numeric" /></label>
      <button type="button" onClick={add}>Add update rule</button>
      <p>To edit a rule, remove it and add its replacement, then Save. This authorizes future matching updates, not just the current proposals.</p>
      <Show when={error()}><p role="alert">{error()}</p></Show>
    </Show>
  </fieldset>;
}

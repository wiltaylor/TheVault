import { For, Show, createEffect, createSignal } from "solid-js";
import { type AiStatus, type ClassifierSettings as Settings, api } from "../api";
import { errorText } from "./TypeEditor";
import { ScamHistory } from "./ScamHistory";

const reasons: Record<string, string> = {
  invalid_definition: "Invalid classifier definition. Check its WCL configuration and filter.",
  invalid_filter: "Invalid classifier filter. Repair the where condition in its WCL file.",
  credential_unavailable: "Cannot read the stored API key. Check the server's secrets key configuration.",
  missing_account: "No matching account. Select an account below.",
  incompatible_kind: "This account cannot run Decisions. Select an OpenAI-compatible /api/v1 account below.",
  invalid_endpoint: "Decisions need an OpenAI-compatible account with an /api/v1 base URL, such as OpenRouter.",
  no_stored_key: "No stored API key. Keyless custom endpoints may work. Add a key in Accounts if your endpoint requires one.",
  locally_ready: "Local setup is ready. Credentials and provider connectivity have not been tested.",
};

export function ClassifierSettings(props: { status: AiStatus }) {
  const [settings, setSettings] = createSignal<Settings>();
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  createEffect(() => {
    void props.status;
    void api.classifierSettings().then(setSettings).catch((e) => setError(errorText(e)));
  });
  async function select(name: string, account: string) {
    if (!account) return;
    setBusy(true);
    setError("");
    try { setSettings(await api.classifierAccount(name, account)); }
    catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  }
  return (
    <section class="card" aria-label="Classifiers">
      <h2>Classifiers</h2>
      <p>Local diagnostics only. Reading this section or changing an account sends no data to a provider and does not retry old notes. The global default account is unchanged.</p>
      <p>Decisions require an OpenAI-compatible /api/v1 endpoint, such as OpenRouter. ChatGPT, direct OpenAI and Anthropic accounts are not supported here. The model comes from the classifier file, not the account's chat or Decisions model setting.</p>
      <Show when={settings()}>{(s) => <For each={s().classifiers}>{(row) => (
        <section aria-label={`${row.name} classifier`}>
          <h3>{row.name}</h3>
          <p>{reasons[row.reason] ?? "Local setup unavailable."}</p>
          <p>Resolved account: {row.resolved_account ?? "none"}. Model: {row.model ?? "unavailable"}.</p>
          <p>Last recorded outcome: {row.last_outcome === "not_recorded" ? "No durable outcome recorded. Older failures are not reconstructed." : row.last_outcome === "attempting" ? "Attempt started. Completion unknown, possibly interrupted. No automatic retry." : row.last_outcome} This is historical, not a live provider test.</p>
          <Show when={row.valid}>
            <label>Account for {row.name}
              <select aria-label={`Account for ${row.name}`} disabled={busy()} value={row.resolved_account_id ?? ""}
                onChange={(e) => void select(row.name, e.currentTarget.value)}>
                <option value="" disabled>Select a Decisions account</option>
                <For each={s().accounts}>{(account) => <option value={account.id}>{account.name}</option>}</For>
              </select>
            </label>
            <Show when={!row.account}><p>Currently inherits the vault default. Choosing an account stores its ID in this classifier only.</p></Show>
          </Show>
          <p>Types: {row.types?.join(", ") ?? "unavailable"}. Filter: {row.filter ?? "none"}.</p>
          <p>{row.name === "agent_safety"
            ? "Agent safety sends the safety envelope, including projected note text, attachment text and provenance, to the selected external account and model. Incoming notes remain held from agents until cleared or explicitly released."
            : `This classifier sends configured fields (${row.fields?.join(", ") ?? "unavailable"}) to the selected external account and model. Scam defaults include title, sender, recipients, snippet and body. Unclassified notes still need triage.`}</p>
          <Show when={row.name === "scam"}><ScamHistory /></Show>
        </section>
      )}</For>}</Show>
      <p>The separate one-time historical safety pass is in <a href="/inbox">Held Inbox</a>. This section does not replay it.</p>
      <Show when={error()}><p role="alert" class="error">{error()}</p></Show>
    </section>
  );
}

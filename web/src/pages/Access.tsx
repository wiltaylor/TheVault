import { For, Show, createResource, createSignal } from "solid-js";
import { type Scope, type Token, api } from "../api";
import { when } from "../admin/format";
import { isName } from "../defs/typeModel";
import { errorText } from "./TypeEditor";

/**
 * Secrets that tasks and after handlers read (D10). A value is written here and never shown
 * again: the API has no way to read one back.
 */
export function Secrets() {
  const [secrets, { refetch }] = createResource(api.secrets);
  const [name, setName] = createSignal("");
  const [value, setValue] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  async function save(ev: SubmitEvent) {
    ev.preventDefault();
    const n = name().trim();
    setError(null);
    setStatus(null);
    if (!isName(n)) {
      setError("A secret name uses a-z, 0-9 and _, and starts with a letter.");
      return;
    }
    setBusy(true);
    try {
      const replaced = secrets()?.some((s) => s.name === n);
      await api.putSecret(n, value());
      setName("");
      setValue("");
      setStatus(replaced ? `Replaced the value of ${n}.` : `Stored ${n}.`);
      await refetch();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove(n: string) {
    if (!confirm(`Delete secret ${n}?`)) return;
    setError(null);
    setStatus(null);
    try {
      await api.deleteSecret(n);
      setStatus(`Deleted ${n}.`);
      await refetch();
    } catch (e) {
      setError(errorText(e));
    }
  }

  return (
    <section class="list-page">
      <header class="note-header">
        <h1>Secrets</h1>
      </header>
      <p class="muted">
        Values are stored encrypted and never shown again. A task or handler names the secrets it may read.
      </p>
      <form class="admin-form" aria-label="Set a secret" onSubmit={save}>
        <div class="form-grid">
          <label for="secret-name">Name</label>
          <input
            id="secret-name"
            value={name()}
            placeholder="digest_webhook"
            autocomplete="off"
            onInput={(e) => setName(e.currentTarget.value)}
          />
          <label for="secret-value">Value</label>
          <input
            id="secret-value"
            type="password"
            autocomplete="new-password"
            value={value()}
            onInput={(e) => setValue(e.currentTarget.value)}
          />
        </div>
        <button type="submit" class="primary" disabled={busy()}>
          Save secret
        </button>
      </form>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={status()}>
        <p role="status">{status()}</p>
      </Show>
      <Show when={secrets()}>
        {(list) => (
          <Show when={list().length > 0} fallback={<p class="muted">No secrets yet.</p>}>
            <table class="admin-table" aria-label="Secrets">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Created</th>
                  <th>Updated</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                <For each={list()}>
                  {(s) => (
                    <tr data-secret={s.name}>
                      <td>
                        <code>{s.name}</code>
                      </td>
                      <td>{when(s.created)}</td>
                      <td>{when(s.updated)}</td>
                      <td class="actions">
                        <button type="button" onClick={() => setName(s.name)}>
                          Replace
                        </button>
                        <button type="button" class="danger" onClick={() => void remove(s.name)}>
                          Delete
                        </button>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        )}
      </Show>
    </section>
  );
}

const SCOPES: { scope: Scope; label: string }[] = [
  { scope: "read", label: "Read: query and read notes" },
  { scope: "write", label: "Write: also create, change and delete notes" },
  { scope: "admin", label: "Admin: also types, scripts, tasks, secrets and tokens" },
];

/**
 * API tokens for agents and the CLI (D11). A new token's secret is shown once, on the page that
 * created it; it is not stored in the browser and the API never returns it again.
 */
export function Tokens() {
  const [tokens, { refetch }] = createResource(api.tokens);
  const [name, setName] = createSignal("");
  const [scope, setScope] = createSignal<Scope>("read");
  const [created, setCreated] = createSignal<Token | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  async function create(ev: SubmitEvent) {
    ev.preventDefault();
    setError(null);
    setCreated(null);
    setBusy(true);
    try {
      setCreated(await api.createToken(name().trim(), scope()));
      setName("");
      await refetch();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function revoke(n: string) {
    if (!confirm(`Revoke token ${n}? Anything using it stops working at once.`)) return;
    setError(null);
    try {
      await api.revokeToken(n);
      if (created()?.name === n) setCreated(null);
      await refetch();
    } catch (e) {
      setError(errorText(e));
    }
  }

  return (
    <section class="list-page">
      <header class="note-header">
        <h1>API tokens</h1>
      </header>
      <form class="admin-form" aria-label="Create a token" onSubmit={create}>
        <div class="form-grid">
          <label for="token-name">Name</label>
          <input
            id="token-name"
            value={name()}
            placeholder="laptop-cli"
            autocomplete="off"
            onInput={(e) => setName(e.currentTarget.value)}
          />
          <label for="token-scope">Scope</label>
          <select id="token-scope" onChange={(e) => setScope(e.currentTarget.value as Scope)}>
            <For each={SCOPES}>
              {(s) => (
                <option value={s.scope} selected={scope() === s.scope}>
                  {s.label}
                </option>
              )}
            </For>
          </select>
        </div>
        <button type="submit" class="primary" disabled={busy() || !name().trim()}>
          Create token
        </button>
      </form>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={created()}>
        {(t) => (
          <div class="token-secret" role="region" aria-label="New token secret">
            <p>
              Copy the secret for <strong>{t().name}</strong> now. It is not shown again.
            </p>
            <div class="actions">
              <input readonly value={t().secret ?? ""} aria-label="Token secret" onFocus={(e) => e.currentTarget.select()} />
              <button type="button" onClick={() => void navigator.clipboard?.writeText(t().secret ?? "")}>
                Copy
              </button>
              <button type="button" onClick={() => setCreated(null)}>
                Done
              </button>
            </div>
          </div>
        )}
      </Show>
      <Show when={tokens()}>
        {(list) => (
          <Show when={list().length > 0} fallback={<p class="muted">No tokens yet.</p>}>
            <table class="admin-table" aria-label="Tokens">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Scope</th>
                  <th>Created</th>
                  <th>Last used</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                <For each={list()}>
                  {(t) => (
                    <tr data-token={t.name}>
                      <td>
                        <code>{t.name}</code>
                      </td>
                      <td>{t.scope}</td>
                      <td>{when(t.created)}</td>
                      <td>{t.last_used === null ? "Never" : when(t.last_used)}</td>
                      <td>
                        <button type="button" class="danger" onClick={() => void revoke(t.name)}>
                          Revoke
                        </button>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        )}
      </Show>
    </section>
  );
}

import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { ApiError, type GitSyncReport, type GitSyncSettings, type GitSyncStatus, api } from "../api";
import { sizeText } from "../assetLibrary";
import { copyText } from "../clipboard";
import { Spinner } from "../components/SyncStatus";
import { errorText } from "./TypeEditor";

// Settings > Sync (git-sync D18): the git sync between this instance and the owner's other
// instances, through a git repo. The settings and every credential are secrets on the server, so
// this page and `vault gitsync` are their only editors. The token and the SSH private key are never
// shown back, only whether each is set; the shared sync key is shown once, when it is generated,
// and is gone on reload (git-sync D14). A 400 from saving names the query or cron it refused, and
// its message shows under that field. Sync now starts a round; the page then reads the status every
// few seconds while the round runs.

/** How often the page reads the status again while a round runs. */
export const POLL_MS = 3000;

/** Where the setup is written down. */
export const GITSYNC_DOC = "https://github.com/wiltaylor/TheVault/blob/main/docs/gitsync.md";

const MB = 1024 * 1024;

/** The settings before any are saved, as the server defaults them. */
export const DEFAULT_SETTINGS: GitSyncSettings = {
  remote_url: "",
  auth: "https",
  https_username: "x-access-token",
  export_query: "",
  import_query: "",
  schedule: "*/15 * * * *",
  name: "",
  warn_bytes: 1024 * MB,
  max_bytes: 4096 * MB,
  paused: false,
};

/** The fields a refused save can name, each shown under its own input. */
type Named = "export_query" | "import_query" | "schedule";

/**
 * The field a 400 from saving the settings is about: the body's `field` when the server names one,
 * else the field its message mentions, else null.
 */
export function refusedField(e: unknown): Named | null {
  if (!(e instanceof ApiError) || e.status !== 400) return null;
  const field = e.body.field;
  if (field === "export_query" || field === "import_query" || field === "schedule") return field;
  const message = (e.body.message ?? "").toLowerCase();
  if (/export[ _]query/.test(message)) return "export_query";
  if (/import[ _]query/.test(message)) return "import_query";
  if (/cron|schedule/.test(message)) return "schedule";
  return null;
}

function whenText(iso: string | null): string {
  if (!iso) return "Never";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

/** A report's count: a number, or the length of the list it counts. */
function count(v: number | unknown[] | undefined): number {
  return Array.isArray(v) ? v.length : (v ?? 0);
}

/** One line for a round's report, such as "Pushed. 3 created, 1 updated, 0 merged, …". */
export function reportText(r: GitSyncReport): string {
  const counts = (["created", "updated", "merged", "clashes", "archived", "unarchived"] as const)
    .map((k) => `${count(r[k])} ${k}`)
    .join(", ");
  return `${r.pushed ? "Pushed" : "Nothing to push"}. ${counts}.`;
}

/** A skipped branch as one line, whatever shape the report gives it. */
function skippedText(b: unknown): string {
  if (typeof b === "string") return b;
  if (b && typeof b === "object") {
    const o = b as Record<string, unknown>;
    const who = o.name ?? o.instance ?? o.branch;
    const why = o.reason ?? o.message;
    if (who || why) return [who, why].filter(Boolean).join(": ");
  }
  return JSON.stringify(b);
}

/** The spinner a button shows while its request is in flight. */
function Busy(props: { on: boolean }) {
  return (
    <Show when={props.on}>
      <Spinner />{" "}
    </Show>
  );
}

/** The request in flight: which button started it. */
type Action =
  | "save"
  | "token"
  | "token-clear"
  | "ssh-generate"
  | "ssh-set"
  | "key-generate"
  | "key-set"
  | "run"
  | "disconnect";

/** Settings > Sync. `pollMs` is [`POLL_MS`] unless a test gives another. */
export function GitSync(props: { pollMs?: number }) {
  const [status, setStatus] = createSignal<GitSyncStatus | null>(null);
  const [form, setForm] = createSignal<GitSyncSettings>(DEFAULT_SETTINGS);
  const [token, setToken] = createSignal("");
  const [privateKey, setPrivateKey] = createSignal("");
  const [pastedKey, setPastedKey] = createSignal("");
  /** The sync key just generated: shown until the page is left, never fetched again. */
  const [newKey, setNewKey] = createSignal<string | null>(null);
  const [copied, setCopied] = createSignal<string | null>(null);
  const [refused, setRefused] = createSignal<{ field: Named; message: string } | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<Action | null>(null);
  /** Whether Sync now was pressed and the page waits to say the round ended. */
  const [watching, setWatching] = createSignal(false);
  const busy = () => pending() !== null;
  const doing = (a: Action) => pending() === a;
  const running = () => !!status()?.running || doing("run");
  const edit = <K extends keyof GitSyncSettings>(key: K, value: GitSyncSettings[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  /** Read the status; with `reset`, put the saved settings back in the form too. */
  async function load(reset = false) {
    try {
      const s = await api.gitsyncStatus();
      setStatus(s);
      if (reset) setForm({ ...DEFAULT_SETTINGS, ...(s.settings ?? {}) });
      if (watching() && !s.running) {
        setWatching(false);
        if (!s.last_error) setNotice("The round finished.");
      }
    } catch (e) {
      setError(errorText(e));
    }
  }
  onMount(() => void load(true));

  // While a round runs, read the status again every few seconds, until it ends.
  createEffect(() => {
    if (!status()?.running) return;
    const timer = setTimeout(() => void load(), props.pollMs ?? POLL_MS);
    onCleanup(() => clearTimeout(timer));
  });

  async function act(action: Action, f: () => Promise<string | null | void>) {
    setError(null);
    setNotice(null);
    setPending(action);
    try {
      const said = await f();
      if (said) setNotice(said);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPending(null);
    }
  }

  function save(ev: SubmitEvent) {
    ev.preventDefault();
    setRefused(null);
    void act("save", async () => {
      try {
        await api.gitsyncSaveSettings(form());
      } catch (e) {
        const field = refusedField(e);
        if (!field) throw e;
        setRefused({ field, message: errorText(e) });
        return null;
      }
      await load(true);
      return "Saved. The next round uses these settings.";
    });
  }

  function saveToken() {
    const t = token().trim();
    if (!t) return;
    void act("token", async () => {
      await api.gitsyncPutToken(t);
      setToken("");
      await load();
      return "Token saved.";
    });
  }

  function clearToken() {
    if (!confirm("Remove the token? Rounds over HTTPS fail until another is set.")) return;
    void act("token-clear", async () => {
      await api.gitsyncDeleteToken();
      await load();
      return "Token removed.";
    });
  }

  function generateSshKey() {
    if (status()?.ssh_key_set && !confirm("Replace the key pair? Add the new public key to the host before the next round.")) {
      return;
    }
    void act("ssh-generate", async () => {
      await api.gitsyncGenerateSshKey();
      await load();
      return "Key pair made. Add the public key to the repo as a deploy key with write access.";
    });
  }

  function saveSshKey() {
    const k = privateKey().trim();
    if (!k) return;
    void act("ssh-set", async () => {
      await api.gitsyncPutSshKey(`${k}\n`);
      setPrivateKey("");
      await load();
      return "Private key saved.";
    });
  }

  function generateKey() {
    if (
      status()?.key_set &&
      !confirm("Make a new sync key? Every other instance reports a key mismatch until you paste the new key there.")
    ) {
      return;
    }
    void act("key-generate", async () => {
      const { key } = await api.gitsyncGenerateKey();
      setNewKey(key);
      setCopied(null);
      await load();
    });
  }

  function saveKey() {
    const k = pastedKey().trim();
    if (!k) return;
    void act("key-set", async () => {
      await api.gitsyncPutKey(k);
      setPastedKey("");
      setNewKey(null);
      await load();
      return "Sync key saved.";
    });
  }

  async function copy(text: string, what: string) {
    setCopied((await copyText(text)) ? `${what} copied.` : "Copy failed; select the text and copy it.");
  }

  function runNow() {
    void act("run", async () => {
      try {
        await api.gitsyncRun();
      } catch (e) {
        // A round already running is the one to watch.
        if (!(e instanceof ApiError && e.status === 409)) throw e;
      }
      setWatching(true);
      await load();
      return null;
    });
  }

  function disconnect() {
    if (
      !confirm(
        "Disconnect the git sync? TheVault forgets the settings, the token, the key pair and the sync key. Notes already synced stay.",
      )
    ) {
      return;
    }
    void act("disconnect", async () => {
      await api.gitsyncDisconnect();
      setNewKey(null);
      await load(true);
      return "Disconnected.";
    });
  }

  /** Enter in a secret's own input does its own action, not the settings form's Save. */
  const onEnter = (f: () => void) => (e: KeyboardEvent) => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    f();
  };

  const RefusedUnder = (p: { field: Named; id: string }) => (
    <Show when={refused()?.field === p.field}>
      <p class="error" role="alert" id={p.id}>
        {refused()!.message}
      </p>
    </Show>
  );

  const bytes = () => status()?.last_report?.bytes ?? null;

  return (
    <section class="list-page gitsync-page">
      <header class="note-header">
        <h1>Sync</h1>
      </header>
      <p class="muted">
        Share chosen notes with your other TheVault instances through a git repo.{" "}
        <a href={GITSYNC_DOC} target="_blank" rel="noopener noreferrer">
          How to set up git sync
        </a>
      </p>
      <Show when={status()}>
        {(s) => (
          <>
            <form class="admin-form gitsync-form" aria-label="Git sync settings" novalidate onSubmit={save}>
              <section class="field-card gitsync-card" aria-label="Connection">
                <h2>Connection</h2>
                <div class="form-grid">
                  <label for="gitsync-remote">Remote URL</label>
                  <input
                    id="gitsync-remote"
                    autocomplete="off"
                    placeholder="https://github.com/you/vault-sync.git"
                    value={form().remote_url}
                    onInput={(e) => edit("remote_url", e.currentTarget.value)}
                  />
                  <label for="gitsync-auth">Sign in with</label>
                  <select
                    id="gitsync-auth"
                    value={form().auth}
                    onChange={(e) => edit("auth", e.currentTarget.value as GitSyncSettings["auth"])}
                  >
                    <option value="https">HTTPS with token</option>
                    <option value="ssh">SSH key</option>
                  </select>
                  <Show when={form().auth === "https"}>
                    <label for="gitsync-username">Username</label>
                    <input
                      id="gitsync-username"
                      autocomplete="off"
                      value={form().https_username}
                      onInput={(e) => edit("https_username", e.currentTarget.value)}
                    />
                    <label for="gitsync-token">Token</label>
                    <input
                      id="gitsync-token"
                      type="password"
                      autocomplete="off"
                      placeholder={s().token_set ? "set" : "Paste a token"}
                      value={token()}
                      onInput={(e) => setToken(e.currentTarget.value)}
                      onKeyDown={onEnter(saveToken)}
                    />
                  </Show>
                </div>
                <Show
                  when={form().auth === "https"}
                  fallback={
                    <div class="gitsync-ssh">
                      <Show
                        when={s().ssh_public_key}
                        fallback={<p class="muted">No key pair yet. Make one, or paste a private key.</p>}
                      >
                        {(pub) => (
                          <div class="token-secret" role="region" aria-label="SSH public key">
                            <p>Add this public key to the repo as a deploy key with write access.</p>
                            <p>
                              <input
                                readonly
                                value={pub()}
                                aria-label="Public key"
                                onFocus={(e) => e.currentTarget.select()}
                              />
                              <button type="button" onClick={() => void copy(pub(), "Public key")}>
                                Copy public key
                              </button>
                            </p>
                          </div>
                        )}
                      </Show>
                      <p class="muted">
                        Host key: {s().host_fingerprint ?? "not pinned yet; the first round over SSH pins it."}
                      </p>
                      <div class="actions">
                        <button
                          type="button"
                          disabled={busy()}
                          aria-busy={doing("ssh-generate") ? "true" : undefined}
                          onClick={generateSshKey}
                        >
                          <Busy on={doing("ssh-generate")} />
                          {s().ssh_key_set ? "Make a new key pair" : "Make a key pair"}
                        </button>
                      </div>
                      <label for="gitsync-private-key">Or paste an OpenSSH private key</label>
                      <textarea
                        id="gitsync-private-key"
                        rows={4}
                        autocomplete="off"
                        spellcheck={false}
                        placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
                        value={privateKey()}
                        onInput={(e) => setPrivateKey(e.currentTarget.value)}
                      />
                      <div class="actions">
                        <button
                          type="button"
                          disabled={busy() || !privateKey().trim()}
                          aria-busy={doing("ssh-set") ? "true" : undefined}
                          onClick={saveSshKey}
                        >
                          <Busy on={doing("ssh-set")} />
                          Save private key
                        </button>
                      </div>
                    </div>
                  }
                >
                  <Show when={s().token_set}>
                    <p class="muted">A token is set. Paste another to replace it.</p>
                  </Show>
                  <div class="actions">
                    <button
                      type="button"
                      disabled={busy() || !token().trim()}
                      aria-busy={doing("token") ? "true" : undefined}
                      onClick={saveToken}
                    >
                      <Busy on={doing("token")} />
                      Save token
                    </button>
                    <Show when={s().token_set}>
                      <button
                        type="button"
                        class="danger"
                        disabled={busy()}
                        aria-busy={doing("token-clear") ? "true" : undefined}
                        onClick={clearToken}
                      >
                        <Busy on={doing("token-clear")} />
                        Remove token
                      </button>
                    </Show>
                  </div>
                </Show>
              </section>

              <section class="field-card gitsync-card" aria-label="Sync key">
                <h2>Sync key</h2>
                <p class="muted">
                  Every instance that syncs through the repo holds the same key. Generate it on the first instance and
                  paste it into the others.
                </p>
                <p>{s().key_set ? "A sync key is set." : "No sync key yet."}</p>
                <Show when={newKey()}>
                  {(k) => (
                    <div class="token-secret" role="region" aria-label="New sync key">
                      <p>Copy the sync key now and paste it into your other instances. It is not shown again.</p>
                      <p>
                        <input readonly value={k()} aria-label="Generated sync key" onFocus={(e) => e.currentTarget.select()} />
                        <button type="button" onClick={() => void copy(k(), "Sync key")}>
                          Copy sync key
                        </button>
                      </p>
                    </div>
                  )}
                </Show>
                <div class="actions">
                  <button
                    type="button"
                    disabled={busy()}
                    aria-busy={doing("key-generate") ? "true" : undefined}
                    onClick={generateKey}
                  >
                    <Busy on={doing("key-generate")} />
                    {s().key_set ? "Generate a new key" : "Generate key"}
                  </button>
                </div>
                <div class="form-grid">
                  <label for="gitsync-paste-key">Paste a key</label>
                  <input
                    id="gitsync-paste-key"
                    type="password"
                    autocomplete="off"
                    placeholder="tvsync1-…"
                    value={pastedKey()}
                    onInput={(e) => setPastedKey(e.currentTarget.value)}
                    onKeyDown={onEnter(saveKey)}
                  />
                </div>
                <div class="actions">
                  <button
                    type="button"
                    disabled={busy() || !pastedKey().trim()}
                    aria-busy={doing("key-set") ? "true" : undefined}
                    onClick={saveKey}
                  >
                    <Busy on={doing("key-set")} />
                    Save key
                  </button>
                </div>
              </section>

              <section class="field-card gitsync-card" aria-label="Queries">
                <h2>Queries</h2>
                <label for="gitsync-export">Export query</label>
                <p class="muted">The notes this instance publishes to the repo.</p>
                <textarea
                  id="gitsync-export"
                  rows={3}
                  spellcheck={false}
                  placeholder='from project where tags == "shared"'
                  aria-invalid={refused()?.field === "export_query" ? "true" : undefined}
                  value={form().export_query}
                  onInput={(e) => edit("export_query", e.currentTarget.value)}
                />
                <RefusedUnder field="export_query" id="gitsync-export-error" />
                <label for="gitsync-import">Import query</label>
                <p class="muted">
                  Which of the other instances' published notes this instance takes. It cannot use <code>this</code>.
                </p>
                <textarea
                  id="gitsync-import"
                  rows={3}
                  spellcheck={false}
                  placeholder="from project"
                  aria-invalid={refused()?.field === "import_query" ? "true" : undefined}
                  value={form().import_query}
                  onInput={(e) => edit("import_query", e.currentTarget.value)}
                />
                <RefusedUnder field="import_query" id="gitsync-import-error" />
              </section>

              <section class="field-card gitsync-card" aria-label="Schedule">
                <h2>Schedule</h2>
                <div class="form-grid">
                  <label for="gitsync-schedule">Schedule (cron)</label>
                  <input
                    id="gitsync-schedule"
                    autocomplete="off"
                    aria-invalid={refused()?.field === "schedule" ? "true" : undefined}
                    value={form().schedule}
                    onInput={(e) => edit("schedule", e.currentTarget.value)}
                  />
                  <span />
                  <RefusedUnder field="schedule" id="gitsync-schedule-error" />
                  <label for="gitsync-paused">Paused</label>
                  <input
                    id="gitsync-paused"
                    type="checkbox"
                    checked={form().paused}
                    onChange={(e) => edit("paused", e.currentTarget.checked)}
                  />
                  <label for="gitsync-name">Instance name</label>
                  <input
                    id="gitsync-name"
                    autocomplete="off"
                    placeholder="The vault folder's name"
                    value={form().name}
                    onInput={(e) => edit("name", e.currentTarget.value)}
                  />
                  <label for="gitsync-warn">Warn above (MB)</label>
                  <input
                    id="gitsync-warn"
                    type="number"
                    min="1"
                    value={Math.round(form().warn_bytes / MB)}
                    onInput={(e) => edit("warn_bytes", Math.round(Number(e.currentTarget.value) * MB))}
                  />
                  <label for="gitsync-max">Refuse above (MB)</label>
                  <input
                    id="gitsync-max"
                    type="number"
                    min="1"
                    value={Math.round(form().max_bytes / MB)}
                    onInput={(e) => edit("max_bytes", Math.round(Number(e.currentTarget.value) * MB))}
                  />
                </div>
                <button type="submit" class="primary" disabled={busy()} aria-busy={doing("save") ? "true" : undefined}>
                  <Busy on={doing("save")} />
                  Save settings
                </button>
              </section>
            </form>

            <section class="field-card gitsync-card" aria-label="Status">
              <h2>Status</h2>
              <dl class="form-grid">
                <dt>Last run</dt>
                <dd>
                  <Show when={running()} fallback={whenText(s().last_run)}>
                    <span class="gitsync-syncing" role="status" aria-busy="true">
                      <Spinner /> Syncing…
                    </span>
                  </Show>
                </dd>
                <dt>Last error</dt>
                <dd classList={{ error: !!s().last_error }}>{s().last_error ?? "No errors"}</dd>
                <Show when={s().last_report}>
                  {(r) => (
                    <>
                      <dt>Last report</dt>
                      <dd>
                        {reportText(r())}
                        <For each={r().skipped_branches ?? []}>
                          {(b) => <div class="muted">Skipped {skippedText(b)}</div>}
                        </For>
                        <For each={r().warnings ?? []}>{(w) => <div class="error">{w}</div>}</For>
                      </dd>
                    </>
                  )}
                </Show>
                <dt>Published size</dt>
                <dd classList={{ error: (bytes() ?? 0) > form().warn_bytes }}>
                  {bytes() === null ? "Not published yet" : sizeText(bytes()!)} (warns above {sizeText(form().warn_bytes)},
                  refused above {sizeText(form().max_bytes)})
                </dd>
              </dl>
              <div class="actions">
                <button
                  type="button"
                  class="primary"
                  disabled={busy() || running() || !s().configured}
                  aria-busy={doing("run") ? "true" : undefined}
                  onClick={runNow}
                >
                  <Busy on={doing("run")} />
                  Sync now
                </button>
                <Show when={s().configured || s().key_set || s().token_set || s().ssh_key_set}>
                  <button
                    type="button"
                    class="danger"
                    disabled={busy()}
                    aria-busy={doing("disconnect") ? "true" : undefined}
                    onClick={disconnect}
                  >
                    <Busy on={doing("disconnect")} />
                    Disconnect
                  </button>
                </Show>
              </div>
            </section>
          </>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={notice()}>
        <p class="muted" role="status">
          {notice()}
        </p>
      </Show>
      <Show when={copied()}>
        <p class="muted">{copied()}</p>
      </Show>
    </section>
  );
}

// The Outbox (outbox X7, X8, X13, X14): emails, SMS and calendar invites waiting to be sent, in
// three tabs: Draft, Ready (ready and sending) and Error. What an agent drafts waits in Draft, as
// what the user composes does, until the user marks it Ready. Every action writes the item's note,
// which syncs, and the server's `outbox_send` handler sends what becomes `ready`. A sent item
// leaves the Outbox: its copy is a regular email, message or event.

import { useNavigate, useSearchParams } from "@solidjs/router";
import { For, type JSX, Show, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { type Device, type Json, type Note, type Row, api } from "../api";
import { changes, onNoteChangedElsewhere } from "../changes";
import { showToast } from "../components/Toasts";
import { noteTitle } from "../editor/titles";
import { OutboxChips } from "../outbox/Chips";
import { createDraft, setItemStatus } from "../outbox/actions";
import { type Kind, type Status, TABS, bodyText, eventChanges, kindOf, newDraft, statusOf, statusQuery, str, strs, tabByKey, tabOf } from "../outbox/model";
import { refreshOutboxCount } from "../outbox/store";
import { errorText } from "./TypeEditor";

const PAGE = 50;

export function Outbox() {
  const navigate = useNavigate();
  // `?item=`: the item to show, as the chat's draft cards link to it, in the tab of its status.
  const [params] = useSearchParams();
  const focus = () => (typeof params.item === "string" ? params.item : "");
  const [tab, setTab] = createSignal(0);
  // `?tab=`: a tab by its key, as SMS Reply all and the drafts notification open Draft.
  createEffect(() => {
    const at = tabByKey(params.tab);
    if (at >= 0) setTab(at);
  });
  createEffect(() => {
    const id = focus();
    if (!id) return;
    void api
      .note(id)
      .then((n) => setTab(tabOf(statusOf(n.note.fields))))
      .catch(() => {});
  });
  const [limit, setLimit] = createSignal(PAGE);
  const [error, setError] = createSignal<string | null>(null);
  // Changes made elsewhere: an agent's drafts, a send finishing, an item marked ready in the apps.
  const [elsewhere, setElsewhere] = createSignal(0);
  onCleanup(onNoteChangedElsewhere(() => setElsewhere((n) => n + 1)));
  // Items acted on, hidden until the list loads again without them.
  const [gone, setGone] = createSignal<Set<string>>(new Set());
  const current = () => TABS[tab()]!;
  const [page] = createResource(
    () => ({ q: statusQuery(current().statuses), n: limit(), v: changes(), e: elsewhere() }),
    async ({ q, n }) => {
      const p = await api.query(q, null, n);
      setGone(new Set<string>());
      return p;
    },
  );
  const [devices] = createResource(() => api.devices().catch(() => [] as Device[]));
  const rows = createMemo(() => (page.latest?.rows ?? []).filter((r) => !gone().has(r.id)));
  const hide = (id: string) => setGone((s) => new Set(s).add(id));
  createEffect(() => {
    tab();
    setLimit(PAGE);
  });

  async function run(id: string | null, act: () => Promise<void>) {
    setError(null);
    try {
      await act();
    } catch (e) {
      setError(errorText(e));
      if (id)
        setGone((s) => {
          const next = new Set(s);
          next.delete(id);
          return next;
        });
    } finally {
      void refreshOutboxCount();
    }
  }

  const move = (r: Row, status: Status, done: string, extra?: Record<string, Json | undefined>) =>
    run(r.id, async () => {
      hide(r.id);
      await setItemStatus(r.id, status, extra);
      showToast(`${done}: “${r.title || "Untitled"}”`);
    });

  const remove = (r: Row) =>
    run(r.id, async () => {
      hide(r.id);
      const n = await api.note(r.id);
      await api.deleteNote(r.id, n.version);
      showToast(`“${r.title || "Untitled"}” moved to the trash`, {
        label: "Undo",
        run: () => void run(r.id, async () => void (await api.restoreFromTrash(r.id))),
      });
    });

  const compose = (kind: Kind) =>
    run(null, async () => {
      const made = await createDraft(newDraft(kind));
      navigate(`/notes/${made.id}`);
    });

  function onTabKey(e: KeyboardEvent) {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const next = (tab() + (e.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length;
    setTab(next);
    document.getElementById(`outbox-tab-${TABS[next]!.key}`)?.focus();
  }

  return (
    <section class="list-page inbox-page outbox-page">
      <header class="note-header inbox-header">
        <h1>Outbox</h1>
        <div class="inbox-tools">
          <button type="button" onClick={() => void compose("email")}>
            New email
          </button>
          <button type="button" onClick={() => void compose("sms")}>
            New SMS
          </button>
          <button type="button" onClick={() => void compose("event")}>
            New event
          </button>
        </div>
      </header>
      <div class="tabs outbox-tabs" role="tablist" aria-label="Outbox" onKeyDown={onTabKey}>
        <For each={TABS}>
          {(t, i) => (
            <button
              type="button"
              role="tab"
              id={`outbox-tab-${t.key}`}
              aria-selected={tab() === i()}
              aria-controls="outbox-panel"
              tabindex={tab() === i() ? 0 : -1}
              onClick={() => setTab(i())}
            >
              {t.label}
            </button>
          )}
        </For>
      </div>
      <div id="outbox-panel" role="tabpanel" aria-labelledby={`outbox-tab-${current().key}`}>
        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>
        <Show when={page.error}>
          <p class="error" role="alert">
            {errorText(page.error)}
          </p>
        </Show>
        <Show when={page.latest}>
          <Show when={rows().length > 0} fallback={<p class="muted inbox-empty">{emptyText(current().key)}</p>}>
            <ul class="inbox-list outbox-list" aria-label={`${current().label} items`}>
              <For each={rows()}>
                {(r) => (
                  <OutboxItem
                    row={r}
                    focused={r.id === focus()}
                    devices={devices() ?? []}
                    onReady={() => void move(r, "ready", "Sending")}
                    onRetry={() => void move(r, "ready", "Sending again", { error: undefined })}
                    onDelete={() => void remove(r)}
                  />
                )}
              </For>
            </ul>
            <Show when={(page.latest?.total ?? 0) > limit()}>
              <button type="button" class="inbox-more" onClick={() => setLimit((n) => n + PAGE)}>
                Show more
              </button>
            </Show>
          </Show>
        </Show>
      </div>
    </section>
  );
}

function emptyText(tab: string): string {
  switch (tab) {
    case "draft":
      return "There are no drafts. Start one with New email, New SMS or New event; what an agent drafts waits here too.";
    case "ready":
      return "Nothing is waiting to be sent. Once sent, an item leaves the Outbox and its copy is a regular email, message or event.";
    default:
      return "No sends failed.";
  }
}

/** Whether the phone still has the SMS item's `send_sms` command to run (outbox X6). */
async function waitingForPhone(commandId: string, device: string, devices: readonly Device[]): Promise<boolean> {
  const ids = device ? [device] : devices.filter((d) => d.platform === "android").map((d) => d.id);
  for (const id of ids) {
    const commands = await api.deviceCommands(id).catch(() => []);
    const c = commands.find((x) => x.id === commandId);
    if (c) return c.status === "pending" || c.status === "delivered";
  }
  return false;
}

function OutboxItem(props: {
  row: Row;
  /** The item the page was opened on, scrolled to and marked. */
  focused?: boolean;
  devices: readonly Device[];
  onReady: () => void;
  onRetry: () => void;
  onDelete: () => void;
}) {
  const [note] = createResource(
    () => props.row.id,
    (id) => api.note(id).catch(() => null),
  );
  const fields = () => note()?.note.fields ?? props.row.fields;
  const kind = () => kindOf(fields());
  const status = () => statusOf(fields());
  const href = () => `/notes/${props.row.id}`;
  const result = () => str(fields(), "result");
  const [waiting] = createResource(
    () => {
      const f = fields();
      const ref = str(f, "sent_ref");
      return kind() === "sms" && ref && !str(f, "sent_at") && (status() === "ready" || status() === "sending")
        ? { ref, device: str(f, "device"), devices: props.devices }
        : null;
    },
    ({ ref, device, devices }) => waitingForPhone(ref, device, devices),
  );
  const statusText = () => (waiting() ? "Waiting for phone" : undefined);
  return (
    <li
      class="inbox-item outbox-item"
      classList={{ "outbox-focused": !!props.focused }}
      aria-current={props.focused ? "true" : undefined}
      data-note={props.row.id}
      data-kind={kind()}
      data-status={status()}
      ref={(el) => {
        if (props.focused) queueMicrotask(() => el.scrollIntoView?.({ block: "nearest" }));
      }}
    >
      <div class="inbox-main">
        <a href={href()} class="inbox-title">
          {props.row.title || "Untitled"}
        </a>
        <p class="muted inbox-meta">
          <span class="outbox-kind">{{ email: "Email", sms: "SMS", event: "Calendar invite" }[kind()]}</span>
          <OutboxChips fields={fields()} statusText={statusText()} />
        </p>
        <Show when={note()}>
          {(n) => (
            <div class="outbox-preview">
              <Show when={kind() === "email"}>
                <EmailPreview note={n()} />
              </Show>
              <Show when={kind() === "sms"}>
                <SmsPreview note={n()} devices={props.devices} />
              </Show>
              <Show when={kind() === "event"}>
                <EventPreview note={n()} />
              </Show>
            </div>
          )}
        </Show>
        <Show when={status() === "failed" && str(fields(), "error")}>
          <p class="error outbox-error">{str(fields(), "error")}</p>
        </Show>
        <Show when={result()}>
          {(id) => (
            <p class="outbox-result">
              <a href={`/notes/${id()}`}>Sent copy: {noteTitle(id()) ?? "open"}</a>
            </p>
          )}
        </Show>
      </div>
      <div class="inbox-actions">
        <Show when={status() === "draft"}>
          <button type="button" class="primary" onClick={props.onReady}>
            Ready
          </button>
        </Show>
        <Show when={status() === "failed"}>
          <button type="button" class="primary" onClick={props.onRetry}>
            Retry
          </button>
        </Show>
        <Show when={status() === "draft" || status() === "failed"}>
          <a href={href()} class="button">
            Edit
          </a>
          <button type="button" class="danger" onClick={props.onDelete}>
            Delete
          </button>
        </Show>
      </div>
    </li>
  );
}

function Header(props: { label: string; children: JSX.Element }) {
  return (
    <div class="outbox-header-row">
      <dt>{props.label}</dt>
      <dd>{props.children}</dd>
    </div>
  );
}

function EmailPreview(props: { note: Note }) {
  const f = () => props.note.note.fields;
  const body = () => bodyText(props.note.note);
  const attachments = () => strs(f(), "attachments");
  const reply = () => str(f(), "in_reply_to");
  return (
    <div class="outbox-email" role="group" aria-label="Email preview">
      <dl class="outbox-headers">
        <Header label="To">{strs(f(), "to").join(", ") || "No one yet"}</Header>
        <Show when={strs(f(), "cc").length}>
          <Header label="Cc">{strs(f(), "cc").join(", ")}</Header>
        </Show>
        <Show when={strs(f(), "bcc").length}>
          <Header label="Bcc">{strs(f(), "bcc").join(", ")}</Header>
        </Show>
        <Header label="Subject">{str(f(), "subject") || "(no subject)"}</Header>
        <Show when={reply()}>
          {(id) => (
            <Header label="In reply to">
              <a href={`/notes/${id()}`}>{noteTitle(id()) ?? "the email"}</a>
            </Header>
          )}
        </Show>
      </dl>
      <Show when={body()}>
        <p class="inbox-preview outbox-body">{body()}</p>
      </Show>
      <Show when={attachments().length}>
        <ul class="outbox-attachments" aria-label="Attachments">
          <For each={attachments()}>{(id) => <Attachment id={id} />}</For>
        </ul>
      </Show>
    </div>
  );
}

function Attachment(props: { id: string }) {
  const [usage] = createResource(
    () => props.id,
    (id) => api.assetUsage(id).catch(() => null),
  );
  return (
    <li>
      <a href={`/assets/${props.id}`}>{usage()?.filename ?? "Attachment"}</a>
    </li>
  );
}

function SmsPreview(props: { note: Note; devices: readonly Device[] }) {
  const f = () => props.note.note.fields;
  const device = () => {
    const id = str(f(), "device");
    if (!id) return "Your phone";
    return props.devices.find((d) => d.id === id)?.name ?? id;
  };
  return (
    <div class="outbox-sms" role="group" aria-label="SMS preview">
      <p class="muted outbox-sms-to">
        To {str(f(), "phone") || "no number yet"} · from {device()}
      </p>
      <p class="outbox-bubble">{bodyText(props.note.note) || "No text yet"}</p>
    </div>
  );
}

function EventPreview(props: { note: Note }) {
  const target = () => str(props.note.note.fields, "event");
  const [event] = createResource(
    () => target() || null,
    (id) => api.note(id).catch(() => null),
  );
  const changes = () => eventChanges(props.note.note, target() ? (event()?.note ?? null) : null);
  return (
    <div class="outbox-event" role="group" aria-label="Event preview">
      <p class="muted">
        <Show when={target()} fallback="A new event">
          {(id) => (
            <>
              A change to <a href={`/notes/${id()}`}>{event()?.note.fields.title ? String(event()!.note.fields.title) : (noteTitle(id()) ?? "the event")}</a>
            </>
          )}
        </Show>
      </p>
      <Show when={changes().length} fallback={<p class="muted">Nothing set yet.</p>}>
        <dl class="outbox-headers outbox-changes">
          <For each={changes()}>
            {(c) => (
              <div class="outbox-header-row" data-field={c.field}>
                <dt>{c.label}</dt>
                <dd>
                  <Show when={target() && c.before !== c.after}>
                    <del class="outbox-before">{c.before || "empty"}</del>
                    {" → "}
                  </Show>
                  <ins class="outbox-after">{c.after}</ins>
                </dd>
              </div>
            )}
          </For>
        </dl>
      </Show>
    </div>
  );
}
